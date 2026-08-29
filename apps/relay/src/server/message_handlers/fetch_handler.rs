// Copyright 2025 The MOQtail Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::server::client::MOQTClient;
use crate::server::message_handlers::parameters;
use crate::server::session_context::{PendingRequest, SessionContext, UpstreamFetchEvent};
use crate::server::stream_id::StreamId;
use core::result::Result::{Err, Ok};
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::Fetch;
use moqtail::model::control::fetch_ok::FetchOk;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::data::fetch_header::FetchHeader;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::StreamResetCode;
use moqtail::model::error::TerminationCode;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::FetchRequest;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

const UPSTREAM_FETCH_CHANNEL_CAPACITY: usize = 64;

/// Why a standalone FETCH range cannot be served as requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchRangeError {
  /// No Objects published, or Start Location beyond the Largest Object.
  InvalidRange,
  /// End Location precedes Start Location.
  ProtocolViolation,
}

/// Resolve a standalone FETCH's requested range against the track's Largest
/// Object (`largest` is None when the track has no published Objects). Returns
/// the End Location to advertise in FETCH_OK, clamped to published data when the
/// request overruns it.
///
/// End Location uses the FETCH encoding: the last Object plus 1, or 0 to mean
/// the entire Group.
pub(crate) fn resolve_standalone_fetch_range(
  start: Location,
  requested_end: Location,
  largest: Option<Location>,
) -> Result<Location, FetchRangeError> {
  // 0 in the Object field means "the entire group", i.e. the largest possible
  // end within that group.
  let effective_end = if requested_end.object == 0 {
    Location::new(requested_end.group, u64::MAX)
  } else {
    requested_end.clone()
  };

  // End Location MUST be the same or larger than Start Location.
  if effective_end < start {
    return Err(FetchRangeError::ProtocolViolation);
  }

  // No Objects published, or Start beyond the Largest Object: INVALID_RANGE.
  let Some(largest) = largest else {
    return Err(FetchRangeError::InvalidRange);
  };
  if start > largest {
    return Err(FetchRangeError::InvalidRange);
  }

  // Clamp End Location to {Largest.Group, Largest.Object + 1} when the request
  // extends beyond published data.
  let clamped_end = Location::new(largest.group, largest.object + 1);
  let end_location = if effective_end > clamped_end {
    clamped_end
  } else {
    requested_end
  };
  Ok(end_location)
}

/// An upstream FETCH issued before FETCH_OK to learn the End Location. It covers the
/// whole requested range, and is handed to the delivery loop so those groups are served
/// from it rather than requested a second time.
pub(crate) struct PendingUpstreamFetch {
  pub relay_request_id: u64,
  pub publisher: Arc<MOQTClient>,
  pub rx: mpsc::Receiver<UpstreamFetchEvent>,
}

/// Whether the relay can answer a standalone FETCH's End Location from what it holds.
///
/// It can when the request ends at or before what it has seen: the clamp cannot bind,
/// so anything the publisher knows beyond that cannot change the answer. Past that
/// point local state would understate the range whenever the relay is behind, and a
/// relay must withhold FETCH_OK until it knows rather than answer early and short.
pub(crate) fn local_state_answers(known: &Option<Location>, requested_end: &Location) -> bool {
  match known {
    Some(known) => requested_end <= &Location::new(known.group, known.object + 1),
    None => false,
  }
}

/// Issues the upstream FETCH for the requested range and waits for its FETCH_OK, so the
/// relay learns the End Location before answering. Updates `largest` from the answer and
/// returns the fetch for the delivery loop to read the Objects from.
///
/// `None` when there is no publisher to ask, or it declines or stays silent; the caller
/// then answers from local state, which is all the relay can honestly claim.
async fn resolve_range_upstream(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  track: &Arc<tokio::sync::RwLock<crate::server::track::Track>>,
  start_group: u64,
  end_group: u64,
  largest: &mut Option<Location>,
) -> Option<PendingUpstreamFetch> {
  let (relay_request_id, publisher, mut rx) = {
    let track_read = track.read().await;
    send_upstream_fetch_for_range(client, context, &track_read, start_group, end_group).await?
  };

  let accepted =
    tokio::time::timeout(context.server_config.upstream_fetch_timeout, rx.recv()).await;

  match accepted {
    Ok(Some(UpstreamFetchEvent::Accepted { end_location })) => {
      // FETCH_OK's End Location is the last Object plus one, where Largest Object is
      // the last Object itself.
      let upstream_largest =
        Location::new(end_location.group, end_location.object.saturating_sub(1));
      if largest
        .as_ref()
        .is_none_or(|known| *known < upstream_largest)
      {
        *largest = Some(upstream_largest);
      }
      Some(PendingUpstreamFetch {
        relay_request_id,
        publisher,
        rx,
      })
    }
    Ok(Some(UpstreamFetchEvent::Error(e))) => {
      warn!("Upstream FETCH {relay_request_id} rejected while resolving the range: {e}");
      None
    }
    Ok(Some(_)) | Ok(None) => {
      warn!("Upstream FETCH {relay_request_id} ended before answering with FETCH_OK");
      None
    }
    Err(_) => {
      warn!("Upstream FETCH {relay_request_id} was not answered in time");
      None
    }
  }
}

/// Why a fetch's object stream is torn down early. A normal cancel closes the
/// stream with a FIN; a failed REQUEST_UPDATE resets it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FetchStop {
  Running,
  Cancelled,
  UpdateFailed,
}

pub async fn handle(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
  opening_request_id: Option<u64>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::Fetch(m) => {
      info!("received Fetch message: {:?}", m);
      let fetch = *m;
      let request_id = fetch.clone().request_id;

      {
        let req = FetchRequest {
          original_request_id: request_id,
          requested_by: client.connection_id,
          fetch_request: fetch.clone(),
          track_alias: 0,
        };

        client
          .incoming_fetch_requests
          .write()
          .await
          .insert(request_id, req.clone());
        client
          .inbound_requests
          .write()
          .await
          .insert(request_id, PendingRequest::Fetch(req));
      }

      // Reserved namespaces are resolved locally and never forwarded upstream.
      if let Some(reason) = crate::server::utils::reserved_namespace_rejection(
        &fetch.track_namespace,
        &fetch.track_name,
      ) {
        info!("Rejecting FETCH for reserved namespace: {}", reason);
        send_request_error(
          client.clone(),
          request_id,
          RequestErrorCode::DoesNotExist,
          ReasonPhrase::try_new(reason.to_string()).unwrap(),
        )
        .await;
        return Ok(());
      }

      let full_track_name = fetch.get_full_track_name();
      let track = context.track_manager.get_track(&full_track_name).await;
      let start_location = fetch.start_location.clone();
      let requested_end = fetch.end_location.clone();

      // TODO: send fetch message to the publisher
      if track.is_none() {
        // TODO: send fetch message to the possible publishers
        // for now just return REQUEST_ERROR
        send_request_error(
          client.clone(),
          request_id,
          RequestErrorCode::DoesNotExist,
          ReasonPhrase::try_new(String::from("Track does not exist")).unwrap(),
        )
        .await;
        return Ok(());
      }

      info!(
        "handle_fetch_messages | Fetching objects from {:?} to {:?}",
        start_location, requested_end
      );

      let track = track.unwrap();
      {
        let track_read = track.read().await;

        let mut inbound = client.inbound_requests.write().await;
        if let Some(PendingRequest::Fetch(req)) = inbound.get_mut(&request_id) {
          req.track_alias = track_read.relay_track_id;
        }
        let mut fetches = client.incoming_fetch_requests.write().await;
        if let Some(req) = fetches.get_mut(&request_id) {
          req.track_alias = track_read.relay_track_id;
        }
      }

      // Validate the requested range and clamp the FETCH_OK End Location to
      // published data. Set when the End Location had to be learned from the
      // publisher; the groups it covers are then served from it instead of being
      // fetched again.
      let mut pending_upstream: Option<PendingUpstreamFetch> = None;

      let end_location = {
        let start = start_location.clone();
        let requested_end = requested_end.clone();
        let mut largest = track.read().await.largest_object().await;

        // A relay must not answer until it knows the End Location. Where the request
        // runs past what the relay has seen, the publisher is asked -- with the upstream
        // FETCH the missing groups need anyway, whose FETCH_OK carries the range it
        // will really serve.
        if !local_state_answers(&largest, &requested_end)
          && let Some(pending) = resolve_range_upstream(
            &client,
            &context,
            &track,
            start.group,
            requested_end.group,
            &mut largest,
          )
          .await
        {
          pending_upstream = Some(pending);
        }
        match resolve_standalone_fetch_range(start.clone(), requested_end.clone(), largest.clone())
        {
          Ok(clamped) => clamped,
          Err(FetchRangeError::ProtocolViolation) => {
            warn!(
              "FETCH request {}: End {:?} precedes Start {:?}; closing session (PROTOCOL_VIOLATION)",
              request_id, requested_end, start
            );
            return Err(TerminationCode::ProtocolViolation);
          }
          Err(FetchRangeError::InvalidRange) => {
            // Only authoritative when there is no upstream publisher that could
            // still serve the range; otherwise fall through to upstream fetch.
            if has_upstream_publisher(&context, &track).await {
              requested_end.clone()
            } else {
              warn!(
                "FETCH request {}: INVALID_RANGE (start {:?}, end {:?}, largest {:?})",
                request_id, start, requested_end, largest
              );
              send_request_error(
                client.clone(),
                request_id,
                RequestErrorCode::InvalidRange,
                ReasonPhrase::try_new(String::from("Invalid range")).unwrap(),
              )
              .await;
              return Ok(());
            }
          }
        }
      };

      // Send FetchOk on the request stream before delivering objects.
      {
        let fetch_ok = FetchOk::new(false, end_location.clone(), vec![], vec![]);
        stream_handler
          .send(&ControlMessage::FetchOk(Box::new(fetch_ok)))
          .await?;
      }

      // Register a cancel channel for this fetch request
      let (cancel_tx, cancel_rx) = watch::channel(FetchStop::Running);
      {
        let mut senders = client.fetch_cancel_senders.write().await;
        senders.insert(request_id, cancel_tx);
      }

      let delivery = FetchDelivery {
        request_id,
        start_location,
        end_location,
        group_order: fetch.group_order(),
        pending_upstream,
      };

      tokio::spawn(async move {
        let result = serve_fetch_stream(client.clone(), context, track, delivery, cancel_rx).await;
        // Clean up the unified map since the fetch is done
        client.inbound_requests.write().await.remove(&request_id);
        client
          .incoming_fetch_requests
          .write()
          .await
          .remove(&request_id);

        // Clean up cancel sender
        client
          .fetch_cancel_senders
          .write()
          .await
          .remove(&request_id);
        result
      });

      Ok(())
    }
    ControlMessage::RequestUpdate(_) => {
      let Some(target_request_id) = opening_request_id else {
        return Err(TerminationCode::ProtocolViolation);
      };
      warn!(
        "REQUEST_UPDATE for FETCH request {} cannot be applied; stopping delivery",
        target_request_id
      );
      let cancel_tx = client
        .fetch_cancel_senders
        .write()
        .await
        .remove(&target_request_id);
      if let Some(tx) = cancel_tx {
        let _ = tx.send(FetchStop::UpdateFailed);
      }
      Ok(())
    }
    _ => {
      // no-op
      Ok(())
    }
  }
}

/// Cancel a fetch when its FETCH request stream is reset or closed: signal the
/// serving task to stop and remove the request from the client maps.
pub(crate) async fn cancel_fetch(client: Arc<MOQTClient>, request_id: u64) {
  let cancel_tx = {
    let mut senders = client.fetch_cancel_senders.write().await;
    senders.remove(&request_id)
  };

  {
    client.inbound_requests.write().await.remove(&request_id);
    client
      .incoming_fetch_requests
      .write()
      .await
      .remove(&request_id);
  }

  if let Some(tx) = cancel_tx {
    let _ = tx.send(FetchStop::Cancelled);
    info!("Cancelled fetch delivery for request_id: {}", request_id);
  }
}

/// Send an upstream Fetch to the publisher for a cache gap [gap_start, gap_end].
/// Returns the relay request ID, the publisher client, and an mpsc::Receiver through which
/// upstream objects will be forwarded.
async fn send_upstream_fetch_for_range(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  track_read: &crate::server::track::Track,
  gap_start: u64,
  gap_end: u64,
) -> Option<(u64, Arc<MOQTClient>, mpsc::Receiver<UpstreamFetchEvent>)> {
  // A FETCH goes to one publisher, unlike a SUBSCRIBE: any of the matching ones may
  // serve the range, so the first will do.
  let publisher = {
    context
      .client_manager
      .get_publishers_for_track(&track_read.full_track_name)
      .await
      .into_iter()
      .next()
  };
  let publisher = match publisher {
    Some(p) => p,
    None => {
      info!(
        "send_upstream_fetch_for_range | No publisher found for {:?}",
        &track_read.full_track_name
      );
      return None;
    }
  };

  let relay_request_id = crate::server::session::Session::get_next_relay_request_id(
    context.relay_next_request_id.clone(),
  )
  .await;

  let upstream_fetch = Fetch::new(
    relay_request_id,
    track_read.full_track_name.namespace.clone(),
    track_read.full_track_name.name.clone(),
    Location::new(gap_start, 0),
    Location::new(gap_end, 0),
    parameters::upstream_fetch(),
  );

  let (upstream_tx, upstream_rx) = mpsc::channel(UPSTREAM_FETCH_CHANNEL_CAPACITY);

  // FETCH is Request, First: it opens its own bidirectional stream. Open it before
  // registering the request so a failure here leaves no state behind.
  let (send, recv) = match publisher.connection.open_bi().await {
    Ok(streams) => streams,
    Err(e) => {
      warn!(
        "send_upstream_fetch_for_range | Failed to open upstream fetch stream: {:?}",
        e
      );
      return None;
    }
  };
  let mut upstream = ControlStreamHandler::new(send, recv).with_peer_id(publisher.connection_id);

  {
    // Use the publisher's track alias so handle_uni_stream can resolve the track on the response.
    let publisher_alias = match track_read
      .publisher_aliases
      .read()
      .await
      .get(&publisher.connection_id)
      .copied()
    {
      Some(alias) => alias,
      None => {
        warn!(
          "send_upstream_fetch_for_range | No publisher alias found for connection {}",
          publisher.connection_id
        );
        return None;
      }
    };
    let req = FetchRequest {
      original_request_id: relay_request_id,
      requested_by: client.connection_id,
      fetch_request: upstream_fetch.clone(),
      track_alias: publisher_alias,
    };
    publisher
      .outgoing_fetch_requests
      .write()
      .await
      .insert(relay_request_id, req);
    context
      .upstream_fetch_senders
      .write()
      .await
      .insert(relay_request_id, upstream_tx.clone());
  }

  // Sent only once the sender is registered, so Objects arriving first are not dropped.
  if let Err(e) = upstream
    .send(&ControlMessage::Fetch(Box::new(upstream_fetch)))
    .await
  {
    warn!(
      "send_upstream_fetch_for_range | Failed to send upstream FETCH: {:?}",
      e
    );
    return None;
  }

  // The response can come at any time relative to object delivery, so it is read on a
  // task. Holding the handler keeps the request stream open for the fetch's lifetime —
  // dropping it would reach the publisher as a cancellation.
  let response_tx = upstream_tx;
  tokio::spawn(async move {
    match upstream.next_message().await {
      Ok(ControlMessage::FetchOk(ok)) => {
        info!(
          "Upstream FETCH {} accepted, end location {:?}",
          relay_request_id, ok.end_location
        );
        // The caller may be holding its own FETCH_OK until it learns this.
        let _ = response_tx
          .send(UpstreamFetchEvent::Accepted {
            end_location: ok.end_location.clone(),
          })
          .await;
      }
      Ok(ControlMessage::RequestError(err)) => {
        // Tell the waiting gap loop now instead of letting it sit until its timeout.
        let _ = response_tx
          .send(UpstreamFetchEvent::Error(format!(
            "upstream FETCH {} rejected: {:?}",
            relay_request_id, err.error_code
          )))
          .await;
        return;
      }
      Ok(other) => {
        warn!(
          "Unexpected {:?} on upstream fetch stream {}",
          other.get_type(),
          relay_request_id
        );
        return;
      }
      Err(e) => {
        debug!("Upstream fetch stream {} closed: {:?}", relay_request_id, e);
        return;
      }
    }
    // Stays parked until the publisher closes the request stream.
    let _ = upstream.next_message().await;
  });

  Some((relay_request_id, publisher, upstream_rx))
}

/// Whether a connected publisher could still serve Objects for this track, as
/// its origin or via an announced namespace. Used to decide whether an empty
/// local range is authoritative (INVALID_RANGE) or should be fetched upstream.
async fn has_upstream_publisher(
  context: &Arc<SessionContext>,
  track: &Arc<tokio::sync::RwLock<crate::server::track::Track>>,
) -> bool {
  let full_track_name = track.read().await.full_track_name.clone();

  !context
    .client_manager
    .get_publishers_for_track(&full_track_name)
    .await
    .is_empty()
}

async fn send_request_error(
  client: Arc<MOQTClient>,
  request_id: u64,
  error_code: RequestErrorCode,
  reason_phrase: ReasonPhrase,
) {
  // TODO: Implement this later.
  // Draft 16 requires a retry interval. Setting to 0 (no retries) for now.
  let retry_interval = 0;
  let request_error = RequestError::new(error_code, retry_interval, reason_phrase);

  client
    .send_response(
      request_id,
      ControlMessage::RequestError(Box::new(request_error)),
    )
    .await;
  // Remove the request from the client maps on error
  client.inbound_requests.write().await.remove(&request_id);
  client
    .incoming_fetch_requests
    .write()
    .await
    .remove(&request_id);
}

/// What to deliver on one fetch data stream, and where the Objects come from.
pub(crate) struct FetchDelivery {
  /// Names the stream: the FETCH_HEADER carries it and the subscriber matches it
  /// against the request that asked for these Objects.
  pub request_id: u64,
  pub start_location: Location,
  pub end_location: Location,
  pub group_order: GroupOrder,
  /// An upstream fetch already covering the range, whose Objects are used instead
  /// of asking for them a second time.
  pub pending_upstream: Option<PendingUpstreamFetch>,
}

/// Opens a unidirectional stream beginning with a FETCH_HEADER and writes the
/// requested range onto it, serving each group from the cache and asking upstream
/// for the ones missing. Closes the stream with a FIN once the range is delivered,
/// or resets it when the caller stops the delivery.
pub(crate) async fn serve_fetch_stream(
  client: Arc<MOQTClient>,
  context: Arc<SessionContext>,
  track: Arc<tokio::sync::RwLock<crate::server::track::Track>>,
  delivery: FetchDelivery,
  mut cancel_rx: watch::Receiver<FetchStop>,
) -> Result<(), TerminationCode> {
  let track_read = track.read().await;
  let FetchDelivery {
    request_id,
    start_location,
    end_location,
    group_order,
    mut pending_upstream,
  } = delivery;

  let fetch_header = FetchHeader::new(request_id);
  let stream_id = StreamId::new_fetch(track_read.relay_track_id, request_id);

  let stream_fn = async move |client: Arc<MOQTClient>, stream_id: &StreamId| {
    let stream_result = client
      .open_stream(stream_id, fetch_header.serialize().unwrap(), 0)
      .await;

    match stream_result {
      Ok(send_stream) => Some(send_stream),
      Err(e) => {
        error!("handle_fetch_messages | Error opening stream: {:?}", e);
        None
      }
    }
  };

  let mut object_count = 0;
  let mut upstream_gap_count: u64 = 0;
  let mut send_stream = None;
  let mut stop_reason = FetchStop::Running;
  let mut fetch_prev_ctx: Option<moqtail::model::data::fetch_object::FetchObjectContext> = None;
  let mut group_id = start_location.group;

  while group_id <= end_location.group {
    let reason = *cancel_rx.borrow();
    if reason != FetchStop::Running {
      info!(
        "handle_fetch_messages | Fetch stopped ({:?}) for request_id: {}",
        reason, request_id
      );
      stop_reason = reason;
      break;
    }

    // A fetch already issued to learn the End Location covers this whole range and
    // its Objects are on their way, so the cache is not consulted for it: reading
    // both would deliver some of them twice.
    let cached = if pending_upstream.is_some() {
      None
    } else {
      track_read.cache.get_group(group_id).await
    };

    if let Some(group_objects) = cached {
      // === CACHE HIT ===
      let objects = group_objects.read().await;
      for object in objects.iter() {
        if group_id == start_location.group && object.object_id < start_location.object {
          continue;
        }
        if group_id == end_location.group && object.object_id >= end_location.object {
          break;
        }

        if object_count == 0 {
          send_stream = match stream_fn(client.clone(), &stream_id).await {
            Some(ss) => Some(ss),
            None => {
              client
                .fetch_cancel_senders
                .write()
                .await
                .remove(&request_id);
              return Err(TerminationCode::InternalError);
            }
          };
        }

        let fetch_obj = moqtail::model::data::fetch_object::FetchObject::Object(object.clone());
        let serialized = fetch_obj
          .serialize(fetch_prev_ctx.as_ref(), group_order)
          .unwrap();
        fetch_prev_ctx = fetch_obj.context();

        if let Err(e) = client
          .write_stream_object(
            &stream_id,
            object.object_id,
            serialized,
            send_stream.as_ref().cloned(),
          )
          .await
        {
          error!(
            "handle_fetch_messages | Error writing object to stream: {:?}",
            e
          );
          client
            .fetch_cancel_senders
            .write()
            .await
            .remove(&request_id);
          return Err(TerminationCode::InternalError);
        }

        if context.server_config.enable_object_logging {
          let sending_time = crate::server::utils::passed_time_since_start();
          if let Ok(fetch_object) = moqtail::model::data::object::Object::try_from_fetch(
            object.clone(),
            track_read.relay_track_id,
          ) {
            track_read
              .object_logger
              .log_fetch_object(
                track_read.relay_track_id,
                context.connection_id,
                request_id,
                &fetch_object,
                true,
                sending_time,
              )
              .await;
          }
        }

        object_count += 1;
      }
      group_id += 1;
    } else {
      // === CACHE MISS ===
      // Scan ahead to find contiguous gap [gap_start .. gap_end]
      let gap_start: u64 = group_id;
      let mut gap_end = group_id;
      while gap_end < end_location.group && track_read.cache.get_group(gap_end + 1).await.is_none()
      {
        gap_end += 1;
      }

      let max_upstream_fetch_gaps = context.server_config.max_upstream_fetch_gaps;
      if upstream_gap_count >= max_upstream_fetch_gaps {
        warn!(
          "handle_fetch_delivery | Reached max upstream fetch gap limit ({}), skipping gap at group {}",
          max_upstream_fetch_gaps, gap_start
        );
        group_id = gap_end + 1;
        continue;
      }
      upstream_gap_count += 1;

      // Reuse the fetch that was issued to learn the End Location: it already
      // covers the rest of the range, so there is nothing more to ask for.
      // Without one, this is an ordinary cache gap and only the gap is fetched.
      let upstream_rx = match pending_upstream.take() {
        Some(pending) => {
          gap_end = end_location.group;
          Some((pending.relay_request_id, pending.publisher, pending.rx))
        }
        None => {
          send_upstream_fetch_for_range(&client, &context, &track_read, gap_start, gap_end).await
        }
      };

      if let Some((relay_request_id, upstream_publisher, mut rx)) = upstream_rx {
        let timeout = context.server_config.upstream_fetch_timeout;
        loop {
          tokio::select! {
            result = tokio::time::timeout(timeout, rx.recv()) => {
              match result {
                // The downstream End Location is already settled by the time a gap
                // is filled, so this only gets logged.
                Ok(Some(UpstreamFetchEvent::Accepted { end_location })) => {
                  debug!(
                    "Upstream FETCH {} for groups {}..{} accepted, end location {:?}",
                    relay_request_id, gap_start, gap_end, end_location
                  );
                }
                Ok(Some(UpstreamFetchEvent::Object(object))) => {
                  if object_count == 0 {
                    send_stream = match stream_fn(client.clone(), &stream_id).await {
                      Some(ss) => Some(ss),
                      None => {
                        client
                          .fetch_cancel_senders
                          .write()
                          .await
                          .remove(&request_id);
                        return Err(TerminationCode::InternalError);
                      }
                    };
                  }

                  let fetch_obj =
                    moqtail::model::data::fetch_object::FetchObject::Object(object.clone());
                  let serialized =
                    fetch_obj.serialize(fetch_prev_ctx.as_ref(), group_order).unwrap();
                  fetch_prev_ctx = fetch_obj.context();

                  if let Err(e) = client
                    .write_stream_object(
                      &stream_id,
                      object.object_id,
                      serialized,
                      send_stream.as_ref().cloned(),
                    )
                    .await
                  {
                    error!(
                      "handle_fetch_messages | Error writing upstream object to stream: {:?}",
                      e
                    );
                    client
                      .fetch_cancel_senders
                      .write()
                      .await
                      .remove(&request_id);
                    return Err(TerminationCode::InternalError);
                  }

                  if context.server_config.enable_object_logging {
                    let sending_time = crate::server::utils::passed_time_since_start();
                    if let Ok(fetch_object) = moqtail::model::data::object::Object::try_from_fetch(
                      object.clone(),
                      track_read.relay_track_id,
                    ) {
                      track_read
                        .object_logger
                        .log_fetch_object(
                          track_read.relay_track_id,
                          context.connection_id,
                          request_id,
                          &fetch_object,
                          true,
                          sending_time,
                        )
                        .await;
                    }
                  }

                  object_count += 1;
                }
                Ok(Some(UpstreamFetchEvent::StreamClosed)) => {
                  break;
                }
                Ok(Some(UpstreamFetchEvent::Error(e))) => {
                  warn!(
                    "handle_fetch_messages | Upstream fetch error for gap [{}, {}]: {}",
                    gap_start, gap_end, e
                  );
                  break;
                }
                Ok(None) => {
                  break;
                }
                Err(_) => {
                  warn!(
                    "handle_fetch_messages | Upstream fetch timed out for gap [{}, {}]",
                    gap_start, gap_end
                  );
                  break;
                }
              }
            }
            _ = cancel_rx.changed() => {
              let reason = *cancel_rx.borrow();
              info!(
                "handle_fetch_messages | Fetch stopped ({:?}) during upstream fetch for request_id: {}",
                reason, request_id
              );
              stop_reason = reason;
              break;
            }
          }
        }

        // Clean up upstream fetch state
        context
          .upstream_fetch_senders
          .write()
          .await
          .remove(&relay_request_id);
        upstream_publisher
          .outgoing_fetch_requests
          .write()
          .await
          .remove(&relay_request_id);
      }

      group_id = gap_end + 1;
    }
  }

  if stop_reason != FetchStop::Running {
    if let Some(the_stream) = send_stream {
      let mut stream = the_stream.lock().await;
      let result = match stop_reason {
        FetchStop::UpdateFailed => stream.reset(StreamResetCode::Cancelled.to_u64()),
        _ => stream.finish().await,
      };
      if let Err(e) = result {
        error!(
          "handle_fetch_messages | Error closing stream on stop ({:?}): {:?}",
          stop_reason, e
        );
      } else {
        info!(
          "handle_fetch_messages | closed fetch stream on stop ({:?}): {:?}",
          stop_reason, &stream_id
        );
      }
      drop(stream);
      client.remove_stream_by_stream_id(&stream_id).await;
    }
  } else if object_count == 0 {
    // Range is valid but empty: FETCH_OK was already sent on the request
    // stream, so just open the data stream, write the header, and FIN.
    info!(
      "handle_fetch_messages | Empty range for request_id: {}. Sending empty stream.",
      request_id
    );

    if let Some(the_stream) = stream_fn(client.clone(), &stream_id).await {
      let mut stream_lock = the_stream.lock().await;
      if let Err(e) = stream_lock.finish().await {
        error!(
          "handle_fetch_messages | Error closing empty fetch stream: {:?}",
          e
        );
      }
      client.remove_stream_by_stream_id(&stream_id).await;
    }
  } else {
    // close the stream instantly
    if let Some(the_stream) = send_stream {
      // gracefully finish the stream here
      if let Err(e) = the_stream.lock().await.finish().await {
        error!("handle_fetch_messages | Error closing stream: {:?}", e);
        // return Err(TerminationCode::InternalError);
      } else {
        info!("finished fetch stream: {:?}", &stream_id);
      }
      client.remove_stream_by_stream_id(&stream_id).await;
      info!("removed stream from the map {}", stream_id);
    }
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::{FetchRangeError, local_state_answers, resolve_standalone_fetch_range};
  use moqtail::model::common::location::Location;

  fn loc(group: u64, object: u64) -> Location {
    Location::new(group, object)
  }

  #[test]
  fn empty_track_is_invalid_range() {
    // A track with no published Objects (largest = None) -> INVALID_RANGE.
    assert_eq!(
      resolve_standalone_fetch_range(loc(0, 0), loc(5, 0), None),
      Err(FetchRangeError::InvalidRange)
    );
  }

  #[test]
  fn start_beyond_largest_is_invalid_range() {
    // Start Location past the Largest Object -> INVALID_RANGE.
    assert_eq!(
      resolve_standalone_fetch_range(loc(10, 0), loc(12, 1), Some(loc(4, 2))),
      Err(FetchRangeError::InvalidRange)
    );
  }

  #[test]
  fn end_before_start_is_protocol_violation() {
    // End Location earlier than Start Location -> PROTOCOL_VIOLATION.
    assert_eq!(
      resolve_standalone_fetch_range(loc(5, 3), loc(5, 1), Some(loc(9, 0))),
      Err(FetchRangeError::ProtocolViolation)
    );
  }

  #[test]
  fn end_beyond_largest_is_clamped() {
    // Requested end overruns published data -> clamp to {Largest.Group, Largest.Object + 1}.
    assert_eq!(
      resolve_standalone_fetch_range(loc(0, 0), loc(100, 0), Some(loc(4, 2))),
      Ok(loc(4, 3))
    );
  }

  #[test]
  fn end_within_published_data_is_unchanged() {
    // Requested end within published data is echoed back verbatim.
    assert_eq!(
      resolve_standalone_fetch_range(loc(0, 0), loc(3, 1), Some(loc(4, 2))),
      Ok(loc(3, 1))
    );
  }

  #[test]
  fn a_request_within_what_is_held_is_answered_locally() {
    // The clamp cannot bind, so whatever the publisher knows past this is irrelevant.
    assert!(local_state_answers(&Some(loc(4, 2)), &loc(3, 1)));
    assert!(local_state_answers(&Some(loc(4, 2)), &loc(4, 0)));
  }

  #[test]
  fn a_request_ending_exactly_at_the_clamp_is_answered_locally() {
    // End Location is the last Object plus one, so {4,3} is precisely "all of {4,2}"
    // and needs nothing from the publisher.
    assert!(local_state_answers(&Some(loc(4, 2)), &loc(4, 3)));
  }

  #[test]
  fn a_request_past_what_is_held_needs_the_publisher() {
    // One object further is already past it: answering locally would understate.
    assert!(!local_state_answers(&Some(loc(4, 2)), &loc(4, 4)));
    assert!(!local_state_answers(&Some(loc(4, 2)), &loc(5, 0)));
  }

  #[test]
  fn holding_nothing_always_needs_the_publisher() {
    assert!(!local_state_answers(&None, &loc(0, 1)));
  }
}
