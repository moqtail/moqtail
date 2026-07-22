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
use crate::server::session_context::{PendingRequest, SessionContext, UpstreamFetchEvent};
use crate::server::stream_id::StreamId;
use crate::server::utils::build_stream_id;
use core::result::Result::{Err, Ok};
use moqtail::model::common::location::Location;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::Fetch;
use moqtail::model::control::fetch_ok::FetchOk;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::data::fetch_header::FetchHeader;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::StreamResetCode;
use moqtail::model::error::TerminationCode;
use moqtail::model::{common::reason_phrase::ReasonPhrase, control::constant::FetchType};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{FetchRequest, HeaderInfo};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

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

      // Resolves the fetch target. The bool is `rejected`: true when a
      // REQUEST_ERROR was already sent, so the caller must stop without also
      // sending DoesNotExist.
      let fn_ = async {
        if let Some(joining_fetch_props) = fetch.clone().joining_fetch_props {
          let sub_request_id = joining_fetch_props.joining_request_id;

          // Resolve the associated subscription regardless of how it was created
          // (SUBSCRIBE, PUBLISH/PUBLISH_OK or REQUEST_UPDATE).
          let Some((track_lock, subscription)) = context
            .track_manager
            .find_subscription_by_request_id(client.connection_id, sub_request_id)
            .await
          else {
            error!(
              "handle_fetch_messages | Joining fetch subscription not found: request_id={}",
              sub_request_id
            );
            return (None, None, None, false);
          };

          // A Joining Fetch is only permitted when the associated subscription
          // has Forward State 1. REQUEST_UPDATE keeps this state current, so
          // reading it here already reflects any processed update.
          if !subscription.read().await.is_forwarding().await {
            warn!(
              "handle_fetch_messages | Joining fetch on non-forwarding subscription {}; INVALID_RANGE",
              sub_request_id
            );
            send_request_error(
              client.clone(),
              request_id,
              RequestErrorCode::InvalidRange,
              ReasonPhrase::try_new(String::from(
                "Joining fetch requires a forwarding subscription",
              ))
              .unwrap(),
            )
            .await;
            return (None, None, None, true);
          }

          let track = track_lock.read().await;
          let largest_location = track.largest_location.read().await;

          if largest_location.group < joining_fetch_props.joining_start {
            error!(
              "handle_fetch_messages | Joining fetch start location is larger than the track's largest location: {:?} {:?}",
              largest_location, joining_fetch_props.joining_start
            );
            send_request_error(
              client.clone(),
              request_id,
              RequestErrorCode::InvalidRange,
              ReasonPhrase::try_new(String::from("Invalid range")).unwrap(),
            )
            .await;
            return (None, None, None, true);
          }

          let start_group = if fetch.fetch_type == FetchType::RelativeFetch {
            largest_location.group - joining_fetch_props.joining_start
          } else {
            joining_fetch_props.joining_start
          };

          let start_location = Location::new(start_group, 0);
          let end_location = Location::new(largest_location.group, largest_location.object + 1);
          (
            Some(track_lock.clone()),
            Some(start_location),
            Some(end_location),
            false,
          )
        } else {
          // standalone fetch
          let props = fetch.standalone_fetch_props.clone().unwrap();

          // let's see whether the track is in the cache
          let full_track_name = moqtail::model::data::full_track_name::FullTrackName {
            namespace: props.track_namespace.clone(),
            name: props.track_name.clone(),
          };
          let track = context.track_manager.get_track(&full_track_name).await;

          if let Some(track) = track {
            (
              Some(track),
              Some(props.start_location.clone()),
              Some(props.end_location.clone()),
              false,
            )
          } else {
            (None, None, None, false)
          }
        }
      };

      let (track, start_location, end_location, rejected) = fn_.await;

      // A REQUEST_ERROR was already sent during resolution; stop here.
      if rejected {
        return Ok(());
      }

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
        start_location.clone().unwrap(),
        end_location.clone().unwrap()
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

      // Standalone FETCH: validate the requested range and clamp the FETCH_OK
      // End Location to published data (draft 10.12/10.13). Joining fetches
      // resolve their own range above.
      let end_location = if fetch.joining_fetch_props.is_none() {
        let largest = track.read().await.largest_object().await;
        let start = start_location.clone().unwrap();
        let requested_end = end_location.clone().unwrap();
        match resolve_standalone_fetch_range(start.clone(), requested_end.clone(), largest.clone())
        {
          Ok(clamped) => Some(clamped),
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
              end_location
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
      } else {
        end_location
      };

      // Send FetchOk on the request stream before delivering objects.
      {
        let fetch_ok = FetchOk::new(false, end_location.clone().unwrap(), vec![], vec![]);
        stream_handler
          .send(&ControlMessage::FetchOk(Box::new(fetch_ok)))
          .await?;
      }

      // Register a cancel channel for this fetch request
      let (cancel_tx, mut cancel_rx) = watch::channel(FetchStop::Running);
      {
        let mut senders = client.fetch_cancel_senders.write().await;
        senders.insert(request_id, cancel_tx);
      }

      tokio::spawn(async move {
        let track_read = track.read().await;
        let start_location = start_location.unwrap();
        let end_location = end_location.unwrap();

        let fetch_header = FetchHeader::new(request_id);
        let header_info = HeaderInfo::Fetch {
          header: fetch_header,
          fetch_request: fetch.clone(),
        };

        let stream_id = build_stream_id(track_read.relay_track_id, &header_info);

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
        let mut fetch_prev_ctx: Option<moqtail::model::data::fetch_object::FetchObjectContext> =
          None;
        let group_order = fetch.group_order();
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

          if let Some(group_objects) = track_read.cache.get_group(group_id).await {
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

              let fetch_obj =
                moqtail::model::data::fetch_object::FetchObject::Object(object.clone());
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
            while gap_end < end_location.group
              && track_read.cache.get_group(gap_end + 1).await.is_none()
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

            // Issue upstream fetch for the gap
            let upstream_rx = send_upstream_fetch_for_range(
              &client,
              &context,
              &track_read,
              &fetch,
              gap_start,
              gap_end,
            )
            .await;

            if let Some((relay_request_id, upstream_publisher, mut rx)) = upstream_rx {
              let timeout = context.server_config.upstream_fetch_timeout;
              loop {
                tokio::select! {
                  result = tokio::time::timeout(timeout, rx.recv()) => {
                    match result {
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
              context
                .relay_pending_requests
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
        Ok(())
      });

      Ok(())
    }
    ControlMessage::RequestUpdate(m) => {
      warn!(
        "REQUEST_UPDATE for FETCH request {} cannot be applied; stopping delivery",
        m.existing_request_id
      );
      let cancel_tx = client
        .fetch_cancel_senders
        .write()
        .await
        .remove(&m.existing_request_id);
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
#[allow(dead_code)]
async fn send_upstream_fetch_for_range(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  track_read: &crate::server::track::Track,
  original_fetch: &Fetch,
  gap_start: u64,
  gap_end: u64,
) -> Option<(u64, Arc<MOQTClient>, mpsc::Receiver<UpstreamFetchEvent>)> {
  let publisher = {
    let m = context.client_manager.read().await;
    match m
      .get_publisher_by_full_track_name(&track_read.full_track_name)
      .await
    {
      Some(p) => Some(p),
      None => {
        m.get_publisher_by_announced_track_namespace(&track_read.full_track_name.namespace)
          .await
      }
    }
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

  let standalone_props = moqtail::model::control::fetch::StandaloneFetchProps {
    track_namespace: track_read.full_track_name.namespace.clone(),
    track_name: track_read.full_track_name.name.clone(),
    start_location: Location::new(gap_start, 0),
    end_location: Location::new(gap_end, 0),
  };
  let upstream_fetch = Fetch::new_standalone(
    relay_request_id,
    standalone_props,
    original_fetch.parameters.clone(),
  );

  let (upstream_tx, upstream_rx) = mpsc::channel(UPSTREAM_FETCH_CHANNEL_CAPACITY);

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
      .insert(relay_request_id, req.clone());
    context
      .relay_pending_requests
      .write()
      .await
      .insert(relay_request_id, PendingRequest::Fetch(req));
    context
      .upstream_fetch_senders
      .write()
      .await
      .insert(relay_request_id, upstream_tx);
  }

  publisher
    .queue_message(ControlMessage::Fetch(Box::new(upstream_fetch)))
    .await;

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
  let m = context.client_manager.read().await;
  m.get_publisher_by_full_track_name(&full_track_name)
    .await
    .is_some()
    || m
      .get_publisher_by_announced_track_namespace(&full_track_name.namespace)
      .await
      .is_some()
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

#[cfg(test)]
mod tests {
  use super::{FetchRangeError, resolve_standalone_fetch_range};
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
}
