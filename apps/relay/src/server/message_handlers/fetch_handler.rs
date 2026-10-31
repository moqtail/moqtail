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
use crate::server::stream_id::{SendStreamKey, StreamId};
use crate::server::track::{Track, TrackOrigin, TrackStatus};
use core::result::Result::{Err, Ok};
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::Fetch;
use moqtail::model::control::fetch_ok::FetchOk;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::data::constant::ObjectForwardingPreference;
use moqtail::model::data::fetch_header::FetchHeader;
use moqtail::model::data::fetch_object::FetchObjectPayload;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::StreamResetCode;
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::combined_filter::{CombinedFilter, ObjectFields};
use moqtail::model::parameter::location_filter::LocationFilter;
use moqtail::model::parameter::message_parameter::include_properties;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::FetchRequest;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{error, info, trace, warn};

const UPSTREAM_FETCH_CHANNEL_CAPACITY: usize = 64;

/// Why a FETCH range cannot be served as requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchRangeError {
  /// No Objects published, or Start Location beyond the Largest Object.
  InvalidRange,
  /// End Location precedes Start Location.
  ProtocolViolation,
}

/// Resolve a FETCH's Location Filter against the track's Largest Object (`largest`
/// is None when the track has no published Objects). Returns the first and last
/// Locations to send; the last is also the End Location for FETCH_OK.
///
/// A relative start counts back from the Largest Object. Without a filter the
/// range starts at {0, 0}, and a filter without an end stops at the Largest Object.
/// A Group End is all of that group, so its Object is `u64::MAX`.
pub(crate) fn resolve_fetch_range(
  filter: &LocationFilter,
  largest: Option<Location>,
) -> Result<(Location, Location), FetchRangeError> {
  // Only the absolute filters have an end, and their start does not depend on the
  // Largest Object, so a filter that ends before it starts is caught here even when
  // nothing is known about the track.
  if let (Some(start), Some(end)) = (filter.start_location(None), filter.end_location())
    && end < start
  {
    return Err(FetchRangeError::ProtocolViolation);
  }
  // Empty when nothing is published or the start is past the Largest Object.
  filter
    .fetch_range(largest)
    .ok_or(FetchRangeError::InvalidRange)
}

/// FETCH_OK must name a real last Object, so an end of `{group, u64::MAX}` (all of
/// the group) is replaced with the group's last Object: the publisher's End Location
/// if it was asked, otherwise the last Object the relay has cached. If neither is
/// known, the end is returned unchanged.
pub(crate) fn concrete_end_location(
  end: Location,
  upstream_end: Option<&Location>,
  cached_last: Option<u64>,
) -> Location {
  if end.object != u64::MAX {
    return end;
  }
  if let Some(upstream_end) = upstream_end
    && *upstream_end < end
  {
    return upstream_end.clone();
  }
  match cached_last {
    Some(object) => Location::new(end.group, object),
    None => end,
  }
}

/// The highest Object ID the relay holds for `group`, if it holds any of it.
async fn last_cached_object_id(
  track: &Arc<tokio::sync::RwLock<crate::server::track::Track>>,
  group: u64,
) -> Option<u64> {
  let objects = track.read().await.cache.get_group(group).await?;
  let objects = objects.read().await;
  objects.iter().map(|object| object.object_id).max()
}

/// The range to serve when the Largest Object is unknown (the relay holds nothing
/// and the publisher did not answer) but a publisher could still deliver Objects:
/// exactly what was asked for. That is only possible when the filter gives both a
/// start and an end; any other filter needs the Largest Object to mean anything.
fn unresolved_fetch_range(filter: &LocationFilter) -> Option<(Location, Location)> {
  match filter {
    LocationFilter::AbsoluteStartGroupEnd { start, .. }
    | LocationFilter::AbsoluteRange { start, .. } => Some((start.clone(), filter.end_location()?)),
    _ => None,
  }
}

/// An upstream FETCH issued before FETCH_OK to learn the End Location. It covers the
/// whole requested range, and is handed to the delivery loop so those groups are served
/// from it rather than requested a second time.
pub(crate) struct PendingUpstreamFetch {
  pub relay_request_id: u64,
  pub publisher: Arc<MOQTClient>,
  pub rx: mpsc::Receiver<UpstreamFetchEvent>,
}

enum UpstreamFetchResponse {
  Accepted {
    end_location: Location,
  },
  /// The publisher answered with REQUEST_ERROR.
  Rejected(Box<RequestError>),
  /// The request failed without an answer from the publisher.
  Failed(String),
}

/// What asking the publisher settled about a FETCH's range.
enum UpstreamRange {
  /// The publisher accepted, and its FETCH_OK gave the End Location.
  Resolved {
    pending: PendingUpstreamFetch,
    end_location: Location,
  },
  /// The publisher refused the range. Its answer is authoritative.
  Rejected(Box<RequestError>),
  /// Nobody could say: no publisher to ask, no answer in time, or a REDIRECT, which
  /// the relay does not follow.
  Unanswered,
}

type UpstreamFetch = (
  u64,
  Arc<MOQTClient>,
  mpsc::Receiver<UpstreamFetchEvent>,
  oneshot::Receiver<UpstreamFetchResponse>,
);

/// Whether the relay can answer a FETCH's End Location from what it holds.
///
/// It can when the request ends at or before what it has seen: the end cannot be cut
/// back, so anything the publisher knows beyond that cannot change the answer. Past
/// that point local state would understate the range whenever the relay is behind, and
/// a relay must withhold FETCH_OK until it knows rather than answer early and short.
pub(crate) fn local_state_answers(known: &Option<Location>, requested_end: &Location) -> bool {
  known.as_ref().is_some_and(|known| requested_end <= known)
}

/// Issues the upstream FETCH for the requested range and waits for its FETCH_OK, so the
/// relay learns the End Location before answering. On success, updates `largest` from
/// the answer and returns the fetch for the delivery loop to read the Objects from.
///
/// The downstream filter is sent upstream unchanged, so a relative start or a missing
/// end is resolved against the publisher's Largest Object, not the relay's.
async fn resolve_range_upstream(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  track: &Arc<tokio::sync::RwLock<crate::server::track::Track>>,
  filter: LocationFilter,
  group_order: GroupOrder,
  largest: &mut Option<Location>,
) -> UpstreamRange {
  let sent = {
    let track_read = track.read().await;
    send_upstream_fetch_for_range(client, context, &track_read, filter, group_order).await
  };
  let Some((relay_request_id, publisher, rx, response_rx)) = sent else {
    return UpstreamRange::Unanswered;
  };

  let response =
    match tokio::time::timeout(context.server_config.upstream_fetch_timeout, response_rx).await {
      Ok(Ok(response)) => response,
      Ok(Err(_)) => {
        warn!("Upstream FETCH {relay_request_id} ended before answering with FETCH_OK");
        return UpstreamRange::Unanswered;
      }
      Err(_) => {
        warn!("Upstream FETCH {relay_request_id} was not answered in time");
        return UpstreamRange::Unanswered;
      }
    };

  match response {
    UpstreamFetchResponse::Accepted { end_location } => {
      // The publisher will send up to its End Location, so the track reaches at least
      // that far.
      if largest.as_ref().is_none_or(|known| *known < end_location) {
        *largest = Some(end_location.clone());
      }
      UpstreamRange::Resolved {
        pending: PendingUpstreamFetch {
          relay_request_id,
          publisher,
          rx,
        },
        end_location,
      }
    }
    UpstreamFetchResponse::Rejected(err) => {
      warn!(
        "Upstream FETCH {relay_request_id} rejected while resolving the range: {:?}",
        err.error_code
      );
      if err.error_code == RequestErrorCode::Redirect {
        UpstreamRange::Unanswered
      } else {
        UpstreamRange::Rejected(err)
      }
    }
    UpstreamFetchResponse::Failed(e) => {
      warn!("Upstream FETCH {relay_request_id} failed while resolving the range: {e}");
      UpstreamRange::Unanswered
    }
  }
}

/// What to do with a FETCH whose range came out empty against the Largest Object the
/// relay knows: the range to serve anyway, or the error to answer with.
///
/// A publisher that refused the range has said what the track holds, so its error is
/// passed on. Only when nobody could say, and a publisher might still deliver, is the
/// request served as written; a filter without both ends cannot be.
fn empty_range_outcome(
  filter: &LocationFilter,
  upstream_rejection: Option<&RequestError>,
  has_upstream_publisher: bool,
) -> Result<(Location, Location), (RequestErrorCode, ReasonPhrase)> {
  if let Some(err) = upstream_rejection {
    return Err((err.error_code, err.reason_phrase.clone()));
  }
  match unresolved_fetch_range(filter) {
    Some(range) if has_upstream_publisher => Ok(range),
    _ => Err((
      RequestErrorCode::InvalidRange,
      ReasonPhrase::try_new(String::from("Invalid range")).unwrap(),
    )),
  }
}

/// Why a fetch's object stream is torn down early. A normal cancel closes the
/// stream with a FIN; a failed REQUEST_UPDATE resets it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FetchStop {
  Running,
  Cancelled,
  MalformedTrack,
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

      // The relay applies Range Filters itself, so ones it cannot honour are refused
      // rather than dropped.
      let filter = match crate::server::filter_constructor::object_filter(
        &fetch.parameters,
        context.server_config.max_filter_ranges,
      ) {
        Ok(filter) => filter,
        Err(e) => {
          info!("Rejecting FETCH {request_id}: {e}");
          let error = crate::server::filter_constructor::invalid_filter(&e);
          send_request_error(
            client.clone(),
            request_id,
            error.error_code,
            error.reason_phrase,
          )
          .await;
          return Ok(());
        }
      };

      let full_track_name = fetch.get_full_track_name();
      let location_filter = fetch.get_location_filter().clone();
      let requested_end = location_filter.end_location();

      // A track the relay does not have yet is created when a publisher can serve it,
      // so the Objects the upstream FETCH brings in are cached for later fetches.
      let track = match context.track_manager.get_track(&full_track_name).await {
        Some(track) => track,
        None => {
          if context
            .client_manager
            .get_publishers_for_track(&full_track_name)
            .await
            .is_empty()
          {
            send_request_error(
              client.clone(),
              request_id,
              RequestErrorCode::DoesNotExist,
              ReasonPhrase::try_new(String::from("Track does not exist")).unwrap(),
            )
            .await;
            return Ok(());
          }
          let (track, _) = context
            .track_manager
            .get_or_create_track(&full_track_name, |relay_track_id| {
              Track::new(
                relay_track_id,
                full_track_name.clone(),
                context.server_config,
                TrackStatus::Pending,
                TrackOrigin::Fetch,
              )
            })
            .await;
          track
        }
      };

      info!(
        "handle_fetch_messages | Fetching objects for {:?}",
        location_filter
      );

      {
        let req = FetchRequest {
          original_request_id: request_id,
          requested_by: client.connection_id,
          fetch_request: fetch.clone(),
          track_alias: track.read().await.relay_track_id,
        };

        client
          .inbound_requests
          .write()
          .await
          .insert(request_id, PendingRequest::Fetch(req));
      }

      // Validate the requested range and clamp the FETCH_OK End Location to
      // published data. Set when the End Location had to be learned from the
      // publisher; the groups it covers are then served from it instead of being
      // fetched again.
      let mut pending_upstream: Option<PendingUpstreamFetch> = None;

      let (start_location, end_location) = {
        let mut largest = track.read().await.largest_object().await;

        // The relay answers on its own only when the requested end is one it has
        // already seen. Otherwise it may be behind the publisher, so it asks upstream
        // and takes the End Location from the publisher's FETCH_OK.
        let answered_locally = requested_end
          .as_ref()
          .is_some_and(|end| local_state_answers(&largest, end));
        let mut upstream_rejection = None;
        let mut upstream_end = None;
        if !answered_locally {
          match resolve_range_upstream(
            &client,
            &context,
            &track,
            location_filter.clone(),
            fetch.group_order(),
            &mut largest,
          )
          .await
          {
            UpstreamRange::Resolved {
              pending,
              end_location,
            } => {
              pending_upstream = Some(pending);
              upstream_end = Some(end_location);
            }
            UpstreamRange::Rejected(err) => upstream_rejection = Some(err),
            UpstreamRange::Unanswered => {}
          }
        }
        match resolve_fetch_range(&location_filter, largest.clone()) {
          Ok((start, end)) => {
            let cached_last = if end.object == u64::MAX && upstream_end.is_none() {
              last_cached_object_id(&track, end.group).await
            } else {
              None
            };
            (
              start,
              concrete_end_location(end, upstream_end.as_ref(), cached_last),
            )
          }
          Err(FetchRangeError::ProtocolViolation) => {
            warn!(
              "FETCH request {}: End precedes Start in {:?}; closing session (PROTOCOL_VIOLATION)",
              request_id, location_filter
            );
            return Err(TerminationCode::ProtocolViolation);
          }
          Err(FetchRangeError::InvalidRange) => {
            let has_publisher =
              upstream_rejection.is_none() && has_upstream_publisher(&context, &track).await;
            match empty_range_outcome(
              &location_filter,
              upstream_rejection.as_deref(),
              has_publisher,
            ) {
              Ok(range) => range,
              Err((error_code, reason_phrase)) => {
                warn!(
                  "FETCH request {}: {:?} ({:?}, largest {:?})",
                  request_id, error_code, location_filter, largest
                );
                send_request_error(client.clone(), request_id, error_code, reason_phrase).await;
                return Ok(());
              }
            }
          }
        }
      };

      // Send FetchOk on the request stream before delivering objects.
      {
        let track_properties = if include_properties(&fetch.parameters) {
          track.read().await.track_properties.read().await.clone()
        } else {
          Vec::new()
        };
        let fetch_ok = FetchOk::new(false, end_location.clone(), vec![], track_properties);
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
        stream_priority: 0,
        pending_upstream,
        filter,
      };

      tokio::spawn(async move {
        let result = serve_fetch_stream(client.clone(), context, track, delivery, cancel_rx).await;
        // Clean up the unified map since the fetch is done
        client.inbound_requests.write().await.remove(&request_id);

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
  }

  if let Some(tx) = cancel_tx {
    let _ = tx.send(FetchStop::Cancelled);
    info!("Cancelled fetch delivery for request_id: {}", request_id);
  }
}

/// Send an upstream Fetch to the publisher for the range `filter` selects, asking for
/// the groups in `group_order` so they arrive in the order they are written downstream.
/// Returns the upstream request state, including separate channels for the control
/// response and Object events.
async fn send_upstream_fetch_for_range(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  track_read: &crate::server::track::Track,
  filter: LocationFilter,
  group_order: GroupOrder,
) -> Option<UpstreamFetch> {
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

  // Delivery treats anything but Descending as Ascending, so upstream is asked the same.
  let group_order = match group_order {
    GroupOrder::Descending => GroupOrder::Descending,
    GroupOrder::Ascending | GroupOrder::Original => GroupOrder::Ascending,
  };
  let upstream_fetch = Fetch::new_with_location_filter(
    relay_request_id,
    track_read.full_track_name.namespace.clone(),
    track_read.full_track_name.name.clone(),
    filter,
    parameters::upstream_fetch(group_order),
  );

  let (upstream_tx, upstream_rx) = mpsc::channel(UPSTREAM_FETCH_CHANNEL_CAPACITY);
  let (response_tx, response_rx) = oneshot::channel();

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
    // The response stream finds its track by the Full Track Name of this request, so a
    // publisher that has given the relay no alias for the track can still serve it.
    let publisher_alias = track_read
      .publisher_aliases
      .read()
      .await
      .get(&publisher.connection_id)
      .copied()
      .unwrap_or_default();
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
  let response_event_tx = upstream_tx;
  let upstream_publisher = publisher.clone();
  tokio::spawn(async move {
    match upstream.next_message().await {
      Ok(ControlMessage::FetchOk(ok)) => {
        info!(
          "Upstream FETCH {} accepted, end location {:?}",
          relay_request_id, ok.end_location
        );
        let end_location = ok.end_location.clone();
        // Signal range resolution before writing to the potentially full event queue.
        let _ = response_tx.send(UpstreamFetchResponse::Accepted {
          end_location: end_location.clone(),
        });
        let _ = response_event_tx
          .send(UpstreamFetchEvent::Accepted { end_location })
          .await;
      }
      Ok(ControlMessage::RequestError(err)) => {
        let error = format!(
          "upstream FETCH {} rejected: {:?}",
          relay_request_id, err.error_code
        );
        let _ = response_tx.send(UpstreamFetchResponse::Rejected(err));
        // Tell the waiting gap loop now instead of letting it sit until its timeout.
        let _ = response_event_tx
          .send(UpstreamFetchEvent::Error(error))
          .await;
        return;
      }
      Ok(ControlMessage::RequestUpdate(_)) => {
        super::close_on_unexpected_request_update(&upstream_publisher, "FETCH");
        let error = format!(
          "upstream FETCH {} failed: publisher sent REQUEST_UPDATE",
          relay_request_id
        );
        let _ = response_tx.send(UpstreamFetchResponse::Failed(error.clone()));
        let _ = response_event_tx
          .send(UpstreamFetchEvent::Error(error))
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
        trace!("Upstream fetch stream {} closed: {:?}", relay_request_id, e);
        return;
      }
    }
    // Stays parked until the publisher closes the request stream. The relay sent
    // this FETCH, so the publisher has no business updating it.
    if let Ok(ControlMessage::RequestUpdate(_)) = upstream.next_message().await {
      super::close_on_unexpected_request_update(&upstream_publisher, "FETCH");
    }
  });

  Some((relay_request_id, publisher, upstream_rx, response_rx))
}

/// What to ask upstream for to fill the groups `first..=last`, which the cache does
/// not hold, for a request of `start..=end`. Whatever comes back is forwarded as it
/// arrives, so it must not reach outside the request.
///
/// Every group the request covers is whole except its own first and last, so the
/// gap's whole groups are clipped to the request. For a request {5,3}..{9,2}: a gap of
/// group 5 is {5,3} to the end of group 5, a gap of group 7 is all of group 7, a gap
/// of group 9 is {9,0}..{9,2}, and a gap of all of them is the request itself.
fn gap_filter(first: u64, last: u64, start: &Location, end: &Location) -> LocationFilter {
  let gap_start = Location::new(first, 0).max(start.clone());
  let gap_end = Location::new(last, u64::MAX).min(end.clone());
  if gap_end.object == u64::MAX {
    LocationFilter::AbsoluteStartGroupEnd {
      start: gap_start,
      end_group: last,
    }
  } else {
    LocationFilter::AbsoluteRange {
      start: gap_start,
      end: gap_end,
    }
  }
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
}

/// What to deliver on one fetch data stream, and where the Objects come from.
pub(crate) struct FetchDelivery {
  /// Names the stream: the FETCH_HEADER carries it and the subscriber matches it
  /// against the request that asked for these Objects.
  pub request_id: u64,
  pub start_location: Location,
  /// The last Location to send. Object `u64::MAX` is all of that group.
  pub end_location: Location,
  pub group_order: GroupOrder,
  /// QUIC send priority for the stream.
  pub stream_priority: i32,
  /// An upstream fetch already covering the range, whose Objects are used instead
  /// of asking for them a second time.
  pub pending_upstream: Option<PendingUpstreamFetch>,
  /// What the requester's Range Filters let through. Upstream is asked for the whole
  /// range regardless: the filtering is the relay's to do.
  pub filter: CombinedFilter,
}

/// What a cached or fetched Object is filtered on. A datagram has no subgroup.
fn filter_fields(object: &FetchObjectPayload) -> ObjectFields<'_> {
  ObjectFields {
    subgroup_id: (object.forwarding_preference != ObjectForwardingPreference::Datagram)
      .then_some(object.subgroup_id),
    object_id: object.object_id,
    publisher_priority: object.publisher_priority,
    properties: object.properties.as_deref(),
  }
}

/// Opens a unidirectional stream beginning with a FETCH_HEADER and writes the
/// requested range onto it, serving each group from the cache and asking upstream
/// for the ones missing. Closes the stream with a FIN once the range is delivered,
/// or resets it when the caller stops the delivery.
///
/// Reports whether a data stream reached the subscriber at all: an empty range still
/// opens one and FINs it, but a delivery stopped before its first object opens none.
pub(crate) async fn serve_fetch_stream(
  client: Arc<MOQTClient>,
  context: Arc<SessionContext>,
  track: Arc<tokio::sync::RwLock<crate::server::track::Track>>,
  delivery: FetchDelivery,
  mut cancel_rx: watch::Receiver<FetchStop>,
) -> Result<bool, TerminationCode> {
  let track_read = track.read().await;
  let FetchDelivery {
    request_id,
    start_location,
    end_location,
    group_order,
    stream_priority,
    mut pending_upstream,
    filter,
  } = delivery;

  let fetch_header = FetchHeader::new(request_id);
  let stream_id = SendStreamKey::new(
    request_id,
    StreamId::new_fetch(track_read.relay_track_id, request_id),
  );

  let stream_fn = async move |client: Arc<MOQTClient>, stream_key: &SendStreamKey| {
    let stream_result = client
      .open_stream(
        stream_key,
        fetch_header.serialize().unwrap(),
        stream_priority,
      )
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
  // Groups are written in the requested order, and the Objects within each group in
  // ascending order either way. The delta encoding of each Object against the one
  // before it depends on that order, so walking the range the other way fails.
  let descending = group_order == GroupOrder::Descending;
  // The group to serve after the groups `lo..=hi`, or None once the range is done.
  let next_after = |lo: u64, hi: u64| -> Option<u64> {
    if descending {
      (lo > start_location.group).then(|| lo - 1)
    } else {
      (hi < end_location.group).then(|| hi + 1)
    }
  };
  let mut next_group = Some(if descending {
    end_location.group
  } else {
    start_location.group
  });

  while let Some(group_id) = next_group {
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
        if group_id == end_location.group && object.object_id > end_location.object {
          break;
        }
        // Skipped before the stream opens or the delta context moves on: the next
        // Object written is encoded against the last one written.
        if !filter.matches(filter_fields(object)) {
          continue;
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
        let serialized = match fetch_obj.serialize(fetch_prev_ctx.as_ref(), group_order) {
          Ok(bytes) => bytes,
          Err(e) => {
            error!("handle_fetch_messages | Error serializing object: {:?}", e);
            client
              .fetch_cancel_senders
              .write()
              .await
              .remove(&request_id);
            return Err(TerminationCode::InternalError);
          }
        };
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
      next_group = next_after(group_id, group_id);
    } else {
      // === CACHE MISS ===
      // Scan ahead, in the direction of delivery, for the contiguous gap
      // [gap_start .. gap_end]
      let mut gap_start = group_id;
      let mut gap_end = group_id;
      if descending {
        while gap_start > start_location.group
          && track_read.cache.get_group(gap_start - 1).await.is_none()
        {
          gap_start -= 1;
        }
      } else {
        while gap_end < end_location.group
          && track_read.cache.get_group(gap_end + 1).await.is_none()
        {
          gap_end += 1;
        }
      }

      let max_upstream_fetch_gaps = context.server_config.max_upstream_fetch_gaps;
      if upstream_gap_count >= max_upstream_fetch_gaps {
        warn!(
          "handle_fetch_delivery | Reached max upstream fetch gap limit ({}), skipping gap at group {}",
          max_upstream_fetch_gaps, group_id
        );
        next_group = next_after(gap_start, gap_end);
        continue;
      }
      upstream_gap_count += 1;

      // Reuse the fetch that was issued to learn the End Location: it already
      // covers the rest of the range, so there is nothing more to ask for.
      // Without one, this is an ordinary cache gap and only the gap is fetched.
      let upstream_rx = match pending_upstream.take() {
        Some(pending) => {
          gap_start = start_location.group;
          gap_end = end_location.group;
          Some((pending.relay_request_id, pending.publisher, pending.rx))
        }
        None => {
          let gap = gap_filter(gap_start, gap_end, &start_location, &end_location);
          send_upstream_fetch_for_range(&client, &context, &track_read, gap, group_order)
            .await
            .map(|(request_id, publisher, rx, _)| (request_id, publisher, rx))
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
                  trace!(
                    "Upstream FETCH {} for groups {}..{} accepted, end location {:?}",
                    relay_request_id, gap_start, gap_end, end_location
                  );
                }
                Ok(Some(UpstreamFetchEvent::Object(object))) => {
                  if !filter.matches(filter_fields(&object)) {
                    continue;
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
                  let serialized =
                    match fetch_obj.serialize(fetch_prev_ctx.as_ref(), group_order) {
                      Ok(bytes) => bytes,
                      Err(e) => {
                        error!(
                          "handle_fetch_messages | Error serializing upstream object: {:?}",
                          e
                        );
                        client
                          .fetch_cancel_senders
                          .write()
                          .await
                          .remove(&request_id);
                        return Err(TerminationCode::InternalError);
                      }
                    };
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
                Ok(Some(UpstreamFetchEvent::MalformedTrack)) => {
                  warn!(
                    "handle_fetch_messages | Malformed upstream FETCH for gap [{}, {}]",
                    gap_start, gap_end
                  );
                  track_read.remove_from_subscriber_switching_sets().await;
                  if let Err(e) = track_read
                    .notify_publish_done(
                      PublishDoneStatusCode::MalformedTrack,
                      "Malformed track".to_string(),
                    )
                    .await
                  {
                    error!("Failed to end subscriptions for malformed track: {e:?}");
                  }
                  stop_reason = FetchStop::MalformedTrack;
                  break;
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

      if stop_reason != FetchStop::Running {
        break;
      }

      next_group = next_after(gap_start, gap_end);
    }
  }

  // Whether a data stream ever reached the subscriber. A fill is counted towards the
  // subscription's PUBLISH_DONE Stream Count, and the subscriber counts what arrives:
  // a fill cancelled before its first object opens nothing, and counting it there
  // would leave the two disagreeing forever.
  let mut opened = false;

  if stop_reason != FetchStop::Running {
    // The reset is how the subscriber learns the track is malformed, so a stop that
    // arrived before the first Object still opens the stream to carry it.
    if stop_reason == FetchStop::MalformedTrack && send_stream.is_none() {
      send_stream = stream_fn(client.clone(), &stream_id).await;
    }

    if let Some(the_stream) = send_stream {
      opened = true;
      let mut stream = the_stream.lock().await;
      let result = match stop_reason {
        FetchStop::UpdateFailed => stream.reset(StreamResetCode::Cancelled.to_u64()),
        FetchStop::MalformedTrack => stream.reset(StreamResetCode::MalformedTrack.to_u64()),
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
      let _ = client.release_stream(&stream_id).await;
    }
  } else if object_count == 0 {
    // Range is valid but empty: FETCH_OK was already sent on the request
    // stream, so just open the data stream, write the header, and FIN.
    info!(
      "handle_fetch_messages | Empty range for request_id: {}. Sending empty stream.",
      request_id
    );

    if let Some(the_stream) = stream_fn(client.clone(), &stream_id).await {
      opened = true;
      let mut stream_lock = the_stream.lock().await;
      if let Err(e) = stream_lock.finish().await {
        error!(
          "handle_fetch_messages | Error closing empty fetch stream: {:?}",
          e
        );
      }
      let _ = client.release_stream(&stream_id).await;
    }
  } else {
    // close the stream instantly
    if let Some(the_stream) = send_stream {
      opened = true;
      // gracefully finish the stream here
      if let Err(e) = the_stream.lock().await.finish().await {
        error!("handle_fetch_messages | Error closing stream: {:?}", e);
        // return Err(TerminationCode::InternalError);
      } else {
        info!("finished fetch stream: {:?}", &stream_id);
      }
      let _ = client.release_stream(&stream_id).await;
      info!("removed stream from the map {}", stream_id);
    }
  }

  Ok(opened)
}

#[cfg(test)]
mod tests {
  use super::{
    FetchRangeError, concrete_end_location, empty_range_outcome, gap_filter, local_state_answers,
    resolve_fetch_range, unresolved_fetch_range,
  };
  use moqtail::model::common::location::Location;
  use moqtail::model::common::reason_phrase::ReasonPhrase;
  use moqtail::model::control::request_error::RequestError;
  use moqtail::model::error::RequestErrorCode;
  use moqtail::model::parameter::location_filter::LocationFilter;

  fn loc(group: u64, object: u64) -> Location {
    Location::new(group, object)
  }

  fn range(start: Location, end: Location) -> LocationFilter {
    LocationFilter::AbsoluteRange { start, end }
  }

  #[test]
  fn empty_track_is_invalid_range() {
    // A track with no published Objects (largest = None) -> INVALID_RANGE.
    assert_eq!(
      resolve_fetch_range(&range(loc(0, 0), loc(5, 0)), None),
      Err(FetchRangeError::InvalidRange)
    );
  }

  #[test]
  fn start_beyond_largest_is_invalid_range() {
    assert_eq!(
      resolve_fetch_range(&range(loc(10, 0), loc(12, 1)), Some(loc(4, 2))),
      Err(FetchRangeError::InvalidRange)
    );
  }

  #[test]
  fn a_filter_ending_before_it_starts_is_a_protocol_violation() {
    // Rejected even with no Largest Object: the filter alone is invalid.
    assert_eq!(
      resolve_fetch_range(&range(loc(5, 3), loc(5, 1)), None),
      Err(FetchRangeError::ProtocolViolation)
    );
  }

  #[test]
  fn no_filter_fetches_the_whole_track() {
    assert_eq!(
      resolve_fetch_range(&LocationFilter::None, Some(loc(4, 2))),
      Ok((loc(0, 0), loc(4, 2)))
    );
  }

  #[test]
  fn a_filter_without_an_end_stops_at_the_largest_object() {
    assert_eq!(
      resolve_fetch_range(
        &LocationFilter::AbsoluteStart { start: loc(2, 1) },
        Some(loc(4, 2))
      ),
      Ok((loc(2, 1), loc(4, 2)))
    );
  }

  #[test]
  fn a_relative_start_counts_back_from_the_largest_object() {
    assert_eq!(
      resolve_fetch_range(
        &LocationFilter::RelativeStart { start_group: 2 },
        Some(loc(4, 2))
      ),
      Ok((loc(3, 0), loc(4, 2)))
    );
  }

  #[test]
  fn next_object_has_nothing_to_fetch() {
    assert_eq!(
      resolve_fetch_range(&LocationFilter::NextObject, Some(loc(4, 2))),
      Err(FetchRangeError::InvalidRange)
    );
  }

  #[test]
  fn an_end_within_published_data_is_kept() {
    assert_eq!(
      resolve_fetch_range(&range(loc(1, 0), loc(3, 1)), Some(loc(4, 2))),
      Ok((loc(1, 0), loc(3, 1)))
    );
    assert_eq!(
      resolve_fetch_range(
        &LocationFilter::AbsoluteStartGroupEnd {
          start: loc(1, 0),
          end_group: 3,
        },
        Some(loc(4, 2))
      ),
      Ok((loc(1, 0), loc(3, u64::MAX))),
      "a Group End is all of that group"
    );
  }

  #[test]
  fn an_end_past_the_largest_object_is_cut_back_to_it() {
    assert_eq!(
      resolve_fetch_range(&range(loc(0, 0), loc(100, 0)), Some(loc(4, 2))),
      Ok((loc(0, 0), loc(4, 2)))
    );
    assert_eq!(
      resolve_fetch_range(
        &LocationFilter::AbsoluteStartGroupEnd {
          start: loc(0, 0),
          end_group: 4,
        },
        Some(loc(4, 2))
      ),
      Ok((loc(0, 0), loc(4, 2))),
      "the current group may still grow, so it ends at the Largest Object"
    );
  }

  #[test]
  fn only_a_filter_with_both_ends_can_be_served_unresolved() {
    assert_eq!(
      unresolved_fetch_range(&range(loc(1, 0), loc(3, 1))),
      Some((loc(1, 0), loc(3, 1)))
    );
    assert_eq!(
      unresolved_fetch_range(&LocationFilter::AbsoluteStart { start: loc(1, 0) }),
      None
    );
    assert_eq!(unresolved_fetch_range(&LocationFilter::None), None);
  }

  #[test]
  fn a_publishers_refusal_is_passed_on() {
    let refusal = RequestError::new(
      RequestErrorCode::InvalidRange,
      0,
      ReasonPhrase::try_new("nothing there".to_string()).unwrap(),
    );
    let (code, reason) =
      empty_range_outcome(&range(loc(950, 0), loc(960, 0)), Some(&refusal), true).unwrap_err();
    assert_eq!(code, RequestErrorCode::InvalidRange);
    assert_eq!(reason, refusal.reason_phrase);
  }

  #[test]
  fn a_refusal_keeps_the_publishers_error_code() {
    let refusal = RequestError::new(
      RequestErrorCode::DoesNotExist,
      0,
      ReasonPhrase::try_new("gone".to_string()).unwrap(),
    );
    assert_eq!(
      empty_range_outcome(&range(loc(1, 0), loc(2, 0)), Some(&refusal), true)
        .unwrap_err()
        .0,
      RequestErrorCode::DoesNotExist
    );
  }

  #[test]
  fn with_no_answer_a_publisher_may_still_serve_a_range_with_both_ends() {
    assert_eq!(
      empty_range_outcome(&range(loc(1, 0), loc(2, 3)), None, true),
      Ok((loc(1, 0), loc(2, 3)))
    );
  }

  #[test]
  fn with_no_answer_and_no_publisher_the_range_is_invalid() {
    assert_eq!(
      empty_range_outcome(&range(loc(1, 0), loc(2, 3)), None, false)
        .unwrap_err()
        .0,
      RequestErrorCode::InvalidRange
    );
  }

  #[test]
  fn with_no_answer_a_range_without_an_end_is_invalid() {
    assert_eq!(
      empty_range_outcome(
        &LocationFilter::AbsoluteStart { start: loc(1, 0) },
        None,
        true
      )
      .unwrap_err()
      .0,
      RequestErrorCode::InvalidRange
    );
  }

  #[test]
  fn a_request_within_what_is_held_is_answered_locally() {
    // The end cannot be cut back, so whatever the publisher knows past this is irrelevant.
    assert!(local_state_answers(&Some(loc(4, 2)), &loc(3, 1)));
    assert!(local_state_answers(&Some(loc(4, 2)), &loc(3, u64::MAX)));
    assert!(local_state_answers(&Some(loc(4, 2)), &loc(4, 2)));
  }

  #[test]
  fn a_request_past_what_is_held_needs_the_publisher() {
    // One object further is already past it: answering locally would understate.
    assert!(!local_state_answers(&Some(loc(4, 2)), &loc(4, 3)));
    assert!(!local_state_answers(&Some(loc(4, 2)), &loc(5, 0)));
    // All of the current group: it may still be growing at the publisher.
    assert!(!local_state_answers(&Some(loc(4, 2)), &loc(4, u64::MAX)));
  }

  #[test]
  fn holding_nothing_always_needs_the_publisher() {
    assert!(!local_state_answers(&None, &loc(0, 0)));
  }

  #[test]
  fn a_concrete_end_is_kept() {
    assert_eq!(
      concrete_end_location(loc(3, 4), Some(&loc(3, 2)), Some(9)),
      loc(3, 4)
    );
  }

  #[test]
  fn a_completed_group_ends_at_its_last_cached_object() {
    assert_eq!(
      concrete_end_location(loc(0, u64::MAX), None, Some(2)),
      loc(0, 2)
    );
  }

  #[test]
  fn the_publishers_end_location_wins_over_the_cache() {
    assert_eq!(
      concrete_end_location(loc(0, u64::MAX), Some(&loc(0, 5)), Some(2)),
      loc(0, 5)
    );
  }

  #[test]
  fn with_nothing_known_about_the_group_the_end_stays_open() {
    assert_eq!(
      concrete_end_location(loc(0, u64::MAX), None, None),
      loc(0, u64::MAX)
    );
  }

  #[test]
  fn a_gap_is_asked_for_only_as_far_as_the_request_reaches() {
    let (start, end) = (loc(5, 3), loc(9, 2));
    let to_group_end =
      |start: Location, end_group: u64| LocationFilter::AbsoluteStartGroupEnd { start, end_group };

    // The request's first group starts at its Start Location.
    assert_eq!(gap_filter(5, 5, &start, &end), to_group_end(loc(5, 3), 5));
    // A group inside the request is whole.
    assert_eq!(gap_filter(7, 7, &start, &end), to_group_end(loc(7, 0), 7));
    // The request's last group stops at its End Location.
    assert_eq!(gap_filter(9, 9, &start, &end), range(loc(9, 0), loc(9, 2)));
    // A gap across the whole request is the request.
    assert_eq!(gap_filter(5, 9, &start, &end), range(loc(5, 3), loc(9, 2)));
    // A request that ends at the end of its last group asks for whole groups.
    assert_eq!(
      gap_filter(7, 9, &start, &loc(9, u64::MAX)),
      to_group_end(loc(7, 0), 9)
    );
  }
}
