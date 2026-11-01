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
use crate::server::track_cache::FillClaim;
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
use moqtail::model::data::fetch_object::{
  EndOfRangeKind, FetchObject, FetchObjectContext, FetchObjectPayload,
};
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::StreamResetCode;
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::combined_filter::{CombinedFilter, ObjectFields};
use moqtail::model::parameter::location_filter::LocationFilter;
use moqtail::model::parameter::message_parameter::include_properties;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::FetchRequest;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
  pub finished: Arc<AtomicBool>,
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

/// The relay's Request ID, whether the response arrived whole (set by its reader once
/// the fetch stream ends with a FIN), its Objects and markers, and its answer.
type UpstreamFetch = (
  u64,
  Arc<AtomicBool>,
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

/// The groups a FETCH claims while it learns its End Location, and for which it waits
/// on a fill another FETCH already has in flight. An open-ended FETCH is never answered
/// locally, so it neither waits nor claims: its claim would reach every later group and
/// hold them for its whole delivery.
fn fill_groups(filter: &LocationFilter) -> Option<(u64, u64)> {
  let end = filter.end_location()?;
  Some((
    filter.start_location(None).map_or(0, |l| l.group),
    end.group,
  ))
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
  let sent = send_upstream_fetch_for_range(client, context, track, filter, group_order).await;
  let Some((relay_request_id, finished, rx, response_rx)) = sent else {
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
          finished,
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

      // Refused before a track is created for the request or a publisher is asked: sent
      // upstream, this FETCH could make the publisher close the relay's whole session
      // over the subscriber's error.
      if let Err(FetchRangeError::ProtocolViolation) = resolve_fetch_range(&location_filter, None) {
        warn!(
          "FETCH request {}: End precedes Start in {:?}; closing session (PROTOCOL_VIOLATION)",
          request_id, location_filter
        );
        return Err(TerminationCode::ProtocolViolation);
      }

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
      let mut pending_fill = None;

      let (start_location, end_location) = {
        let mut largest = track.read().await.largest_object().await;

        // The relay answers on its own only when the requested end is one it has
        // already seen. Otherwise it may be behind the publisher, so it asks upstream
        // and takes the End Location from the publisher's FETCH_OK.
        let mut answered_locally = requested_end
          .as_ref()
          .is_some_and(|end| local_state_answers(&largest, end));
        // If another FETCH is already asking upstream for these groups, wait for it
        // once: what it brings in may let the relay answer this one itself.
        if !answered_locally
          && let (Some(end), Some((first, last))) = (&requested_end, fill_groups(&location_filter))
        {
          let cache = track.read().await.cache.clone();
          pending_fill = match cache.claim_fill(first, last) {
            Ok(claim) => Some(claim),
            Err(in_flight) => {
              let timeout = context.server_config.upstream_fetch_timeout;
              let _ = tokio::time::timeout(timeout, in_flight.lock()).await;
              largest = track.read().await.largest_object().await;
              answered_locally = local_state_answers(&largest, end);
              cache.claim_fill(first, last).ok()
            }
          };
        }
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
            unreachable!("End before Start is refused before the track is looked up")
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

      // Without an upstream fetch of its own, the delivery's gap fills claim their own
      // groups, and must not find this claim in the way.
      if pending_upstream.is_none() {
        pending_fill = None;
      }

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
        pending_fill,
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
  track: &Arc<tokio::sync::RwLock<crate::server::track::Track>>,
  filter: LocationFilter,
  group_order: GroupOrder,
) -> Option<UpstreamFetch> {
  let (full_track_name, publisher_aliases) = {
    let track = track.read().await;
    (
      track.full_track_name.clone(),
      track.publisher_aliases.clone(),
    )
  };
  // A FETCH goes to one publisher, unlike a SUBSCRIBE: any of the matching ones may
  // serve the range, so the first will do.
  let publisher = {
    context
      .client_manager
      .get_publishers_for_track(&full_track_name)
      .await
      .into_iter()
      .next()
  };
  let publisher = match publisher {
    Some(p) => p,
    None => {
      info!(
        "send_upstream_fetch_for_range | No publisher found for {:?}",
        &full_track_name
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
    full_track_name.namespace.clone(),
    full_track_name.name.clone(),
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
    let publisher_alias = publisher_aliases
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
    forget_upstream_fetch(context, &publisher, relay_request_id).await;
    return None;
  }

  // The response can come at any time relative to object delivery, so it is read on a
  // task. Holding the handler keeps the request stream open for the fetch's lifetime.
  // Whoever reads the Objects is done with the fetch once it drops its receiver, on
  // whatever path: the task then closes the request stream if it is still open, with
  // a FIN when the whole response arrived and a cancel otherwise, and forgets the fetch.
  let response_event_tx = upstream_tx;
  let upstream_publisher = publisher.clone();
  let context = context.clone();
  let finished = Arc::new(AtomicBool::new(false));
  let finished_by_reader = finished.clone();
  tokio::spawn(async move {
    let response = async {
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
    };
    let reader_left = tokio::select! {
      _ = response => false,
      _ = response_event_tx.closed() => true,
    };
    if reader_left && finished_by_reader.load(Ordering::Acquire) {
      // The whole response arrived, so the request is closed rather than cancelled, and
      // a publisher that answers by closing its own side is given a moment to do so.
      upstream.finish().await;
      let _ = tokio::time::timeout(
        context.server_config.upstream_fetch_timeout,
        upstream.next_message(),
      )
      .await;
    } else if reader_left {
      upstream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
    } else {
      // The request stream may close before the fetch stream has delivered everything,
      // so the fetch is remembered until its reader is done.
      response_event_tx.closed().await;
    }
    forget_upstream_fetch(&context, &upstream_publisher, relay_request_id).await;
  });

  Some((relay_request_id, finished, upstream_rx, response_rx))
}

async fn forget_upstream_fetch(
  context: &SessionContext,
  publisher: &MOQTClient,
  relay_request_id: u64,
) {
  context
    .upstream_fetch_senders
    .write()
    .await
    .remove(&relay_request_id);
  publisher
    .outgoing_fetch_requests
    .write()
    .await
    .remove(&relay_request_id);
}

/// The part of the groups `first..=last` to ask upstream for, for a request of
/// `start..=end`. Whatever comes back is forwarded as it arrives, so it must not reach
/// outside the request.
///
/// Every group the request covers is whole except its own first and last, so the
/// gap's whole groups are clipped to the request. For a request {5,3}..{9,2}: a gap of
/// group 5 is {5,3} to the end of group 5, a gap of group 7 is all of group 7, a gap
/// of group 9 is {9,0}..{9,2}, and a gap of all of them is the request itself.
fn gap_range(first: u64, last: u64, start: &Location, end: &Location) -> (Location, Location) {
  (
    Location::new(first, 0).max(start.clone()),
    Location::new(last, u64::MAX).min(end.clone()),
  )
}

/// The first Location a fetch stream for `start..=end` carries. A descending FETCH
/// across groups carries each group whole, the start group included: a gap before the
/// first Object of a group means those Objects do not exist, so leaving out the ones
/// before the Start Location would tell the subscriber they do not.
fn delivered_start(start: Location, end: &Location, group_order: GroupOrder) -> Location {
  if group_order == GroupOrder::Descending && start.group != end.group {
    Location::new(start.group, 0)
  } else {
    start
  }
}

/// The Object IDs of `group` a request of `start..=end` wants, clipped the same way.
fn objects_wanted(group: u64, start: &Location, end: &Location) -> (u64, u64) {
  let (lo, hi) = gap_range(group, group, start, end);
  (lo.object, hi.object)
}

/// The last Location of the groups `first..=last` a request of `start..=end` delivers:
/// the end of the highest group in ascending order, of the lowest in descending.
fn last_delivered(
  first: u64,
  last: u64,
  start: &Location,
  end: &Location,
  descending: bool,
) -> Location {
  let group = if descending { first } else { last };
  gap_range(group, group, start, end).1
}

/// An End of Unknown Range marker up to `location`.
fn unknown_up_to(location: Location) -> FetchObject {
  FetchObject::EndOfRange {
    kind: EndOfRangeKind::Unknown,
    group_id: location.group,
    object_id: location.object,
  }
}

/// The Location filter selecting `start..=end`.
fn range_filter(start: &Location, end: &Location) -> LocationFilter {
  if end.object == u64::MAX {
    LocationFilter::AbsoluteStartGroupEnd {
      start: start.clone(),
      end_group: end.group,
    }
  } else {
    LocationFilter::AbsoluteRange {
      start: start.clone(),
      end: end.clone(),
    }
  }
}

/// An inclusive range of Locations.
type Span = (Location, Location);

/// Works out which Locations an upstream FETCH response shows do not exist, as its
/// Objects and End of Range markers arrive.
///
/// A FETCH response carries nothing for a Location it has no Object for. So every
/// Location it passes over is absent, unless a marker says those Locations are
/// unknown or timed out. The Locations after the last one it carries count only once
/// the stream has closed cleanly after FETCH_OK said where the response ends.
///
/// Entries come in delivery order: by Location, or, for a descending FETCH across
/// groups, in whole groups highest first, the Object IDs in each ascending.
struct UpstreamCoverage {
  /// The first and last Locations the response can carry.
  start: Location,
  end: Location,
  descending: bool,
  accepted: bool,
  /// The last Object or marker that arrived.
  last: Option<Location>,
}

impl UpstreamCoverage {
  fn new(start: Location, end: Location, group_order: GroupOrder) -> Self {
    let descending = group_order == GroupOrder::Descending && start.group != end.group;
    let start = delivered_start(start, &end, group_order);
    Self {
      start,
      end,
      descending,
      accepted: false,
      last: None,
    }
  }

  /// The publisher's End Location: the response may end short of what was asked for,
  /// at the Largest Object, whose group can still be growing.
  fn accepted(&mut self, end_location: &Location) {
    if *end_location < self.end {
      self.end = end_location.clone();
    }
    self.accepted = true;
  }

  fn object(&mut self, location: &Location) -> Vec<Span> {
    let spans = self.between(location);
    self.last = Some(location.clone());
    spans
  }

  /// A marker covers the Locations since the last entry, and its own.
  fn marker(&mut self, kind: EndOfRangeKind, location: &Location) -> Vec<Span> {
    let mut spans = self.between(location);
    spans.push((location.clone(), location.clone()));
    self.last = Some(location.clone());
    if kind == EndOfRangeKind::NonExistent {
      self.clip(spans)
    } else {
      Vec::new()
    }
  }

  /// The Locations after the last entry, once the response is over.
  fn finish(self, complete: bool) -> Vec<Span> {
    if !(complete && self.accepted) {
      return Vec::new();
    }
    let mut spans = Vec::new();
    match (&self.last, self.descending) {
      (None, _) => spans.push((self.start.clone(), self.end.clone())),
      (Some(last), false) => {
        if let Some(next) = last.next() {
          spans.push((next, self.end.clone()));
        }
      }
      (Some(last), true) => {
        // The rest of the last group, then every lower group left.
        if last.object < self.group_end(last.group) {
          spans.push((
            Location::new(last.group, last.object + 1),
            Location::new(last.group, self.group_end(last.group)),
          ));
        }
        if self.start.group < last.group {
          spans.push((self.start.clone(), Location::new(last.group - 1, u64::MAX)));
        }
      }
    }
    self.clip(spans)
  }

  /// The last Object ID of `group` the response can carry.
  fn group_end(&self, group: u64) -> u64 {
    if group == self.end.group {
      self.end.object
    } else {
      u64::MAX
    }
  }

  /// The Locations passed over between the last entry and `to`.
  fn between(&self, to: &Location) -> Vec<Span> {
    let mut spans = Vec::new();
    if !self.descending {
      let from = match &self.last {
        Some(last) => last.next(),
        None => Some(self.start.clone()),
      };
      if let (Some(from), Some(until)) = (from, to.prev()) {
        spans.push((from, until));
      }
      return self.clip(spans);
    }

    let head_of_to = |spans: &mut Vec<Span>| {
      if to.object > 0 {
        spans.push((
          Location::new(to.group, 0),
          Location::new(to.group, to.object - 1),
        ));
      }
    };
    match &self.last {
      Some(last) if last.group == to.group => {
        if let (Some(from), Some(until)) = (last.next(), to.prev()) {
          spans.push((from, until));
        }
      }
      Some(last) => {
        // The rest of the last group, the groups skipped, the start of this one.
        if last.object < self.group_end(last.group) {
          spans.push((
            Location::new(last.group, last.object + 1),
            Location::new(last.group, self.group_end(last.group)),
          ));
        }
        if to.group + 1 < last.group {
          spans.push((
            Location::new(to.group + 1, 0),
            Location::new(last.group - 1, u64::MAX),
          ));
        }
        head_of_to(&mut spans);
      }
      None => {
        // The groups above this one, from the response's highest down.
        if to.group < self.end.group {
          spans.push((Location::new(to.group + 1, 0), self.end.clone()));
        }
        head_of_to(&mut spans);
      }
    }
    self.clip(spans)
  }

  /// Keeps only the non-empty parts of `spans` the response can carry.
  fn clip(&self, spans: Vec<Span>) -> Vec<Span> {
    spans
      .into_iter()
      .map(|(from, to)| (from.max(self.start.clone()), to.min(self.end.clone())))
      .filter(|(from, to)| from <= to)
      .collect()
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
  /// Marks the groups `pending_upstream` is filling, so other FETCHes wait for it
  /// rather than ask for them too. Held until the delivery ends.
  pub pending_fill: Option<FillClaim>,
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
  let (relay_track_id, cache, object_logger) = {
    let track = track.read().await;
    (
      track.relay_track_id,
      track.cache.clone(),
      track.object_logger.clone(),
    )
  };
  let FetchDelivery {
    request_id,
    start_location,
    end_location,
    group_order,
    stream_priority,
    mut pending_upstream,
    pending_fill: _pending_fill,
    filter,
  } = delivery;
  // Served from here on, from the cache and from upstream alike.
  let start_location = delivered_start(start_location, &end_location, group_order);

  let fetch_header = FetchHeader::new(request_id);
  let stream_id = SendStreamKey::new(request_id, StreamId::new_fetch(relay_track_id, request_id));

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

  let mut upstream_gap_count: u64 = 0;
  // The group at which this delivery last waited on another FETCH's fill.
  let mut waited_at: Option<u64> = None;
  let mut send_stream = None;
  let mut stop_reason = FetchStop::Running;
  let mut fetch_prev_ctx: Option<FetchObjectContext> = None;
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
  // What a fill could not bring in is marked unknown: left unmarked, the subscriber
  // would take it for Objects that do not exist.
  let unfilled = |first: u64, last: u64| {
    unknown_up_to(last_delivered(
      first,
      last,
      &start_location,
      &end_location,
      descending,
    ))
  };
  let mut next_group = Some(if descending {
    end_location.group
  } else {
    start_location.group
  });

  // Writes one Object or marker, opening the stream with the first. The next entry is
  // encoded against this one.
  let write_entry = async |entry: FetchObject,
                           send_stream: &mut Option<_>,
                           prev_ctx: &mut Option<FetchObjectContext>|
         -> Result<(), TerminationCode> {
    if send_stream.is_none() {
      *send_stream = Some(
        stream_fn(client.clone(), &stream_id)
          .await
          .ok_or(TerminationCode::InternalError)?,
      );
    }
    let serialized = entry
      .serialize(prev_ctx.as_ref(), group_order)
      .map_err(|e| {
        error!(
          "handle_fetch_messages | Error serializing fetch entry: {:?}",
          e
        );
        TerminationCode::InternalError
      })?;
    *prev_ctx = Some(entry.next_context(prev_ctx.as_ref()));
    let object_id = match &entry {
      FetchObject::Object(object) => object.object_id,
      FetchObject::EndOfRange { object_id, .. } => *object_id,
    };
    client
      .write_stream_object(&stream_id, object_id, serialized, send_stream.clone())
      .await
      .map_err(|e| {
        error!(
          "handle_fetch_messages | Error writing to fetch stream: {:?}",
          e
        );
        TerminationCode::InternalError
      })?;

    if let FetchObject::Object(object) = entry
      && context.server_config.enable_object_logging
    {
      let sending_time = crate::server::utils::passed_time_since_start();
      if let Ok(fetch_object) =
        moqtail::model::data::object::Object::try_from_fetch(object, relay_track_id)
      {
        object_logger
          .log_fetch_object(
            relay_track_id,
            context.connection_id,
            request_id,
            &fetch_object,
            true,
            sending_time,
          )
          .await;
      }
    }
    Ok(())
  };

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

    // A fetch already issued to learn the End Location covers this whole range and its
    // Objects are on their way, so the cache is not consulted for it: reading both
    // would deliver some of them twice. Otherwise a group is served from the cache only
    // if the cache accounts for every Location the range wants from it.
    let cached = if pending_upstream.is_some() {
      None
    } else {
      let (lo, hi) = objects_wanted(group_id, &start_location, &end_location);
      cache.objects_if_complete(group_id, lo, hi).await
    };

    if let Some(objects) = cached {
      // === CACHE HIT ===
      for object in objects.iter() {
        // Skipped before the stream opens or the delta context moves on: the next
        // Object written is encoded against the last one written.
        if !filter.matches(filter_fields(object)) {
          continue;
        }

        write_entry(
          FetchObject::Object(object.clone()),
          &mut send_stream,
          &mut fetch_prev_ctx,
        )
        .await?;
      }
      next_group = next_after(group_id, group_id);
    } else {
      // === CACHE MISS ===
      // Scan ahead, in the direction of delivery, for the contiguous gap
      // [gap_start .. gap_end] of groups the cache does not account for.
      let complete = async |group: u64| {
        let (lo, hi) = objects_wanted(group, &start_location, &end_location);
        cache.objects_if_complete(group, lo, hi).await.is_some()
      };
      let mut gap_start = group_id;
      let mut gap_end = group_id;
      if descending {
        while gap_start > start_location.group && !complete(gap_start - 1).await {
          gap_start -= 1;
        }
      } else {
        while gap_end < end_location.group && !complete(gap_end + 1).await {
          gap_end += 1;
        }
      }

      // If another FETCH is already filling some of these groups, wait for it once and
      // check the cache again, rather than ask upstream for the same Objects. Whatever
      // it leaves missing, such as an unknown range, is then asked for here.
      let _fill = match &pending_upstream {
        Some(_) => None,
        None => match cache.claim_fill(gap_start, gap_end) {
          Ok(claim) => Some(claim),
          Err(in_flight) if waited_at != Some(group_id) => {
            waited_at = Some(group_id);
            let timeout = context.server_config.upstream_fetch_timeout;
            tokio::select! {
              _ = tokio::time::timeout(timeout, in_flight.lock()) => {}
              _ = cancel_rx.changed() => {}
            }
            continue;
          }
          Err(_) => None,
        },
      };

      let max_upstream_fetch_gaps = context.server_config.max_upstream_fetch_gaps;
      if upstream_gap_count >= max_upstream_fetch_gaps {
        warn!(
          "handle_fetch_delivery | Reached max upstream fetch gap limit ({}), skipping gap at group {}",
          max_upstream_fetch_gaps, group_id
        );
        write_entry(
          unfilled(gap_start, gap_end),
          &mut send_stream,
          &mut fetch_prev_ctx,
        )
        .await?;
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
          let coverage =
            UpstreamCoverage::new(start_location.clone(), end_location.clone(), group_order);
          Some((
            pending.relay_request_id,
            pending.finished,
            pending.rx,
            coverage,
          ))
        }
        None => {
          let (start, end) = gap_range(gap_start, gap_end, &start_location, &end_location);
          let gap = range_filter(&start, &end);
          send_upstream_fetch_for_range(&client, &context, &track, gap, group_order)
            .await
            .map(|(request_id, finished, rx, _)| {
              let coverage = UpstreamCoverage::new(start, end, group_order);
              (request_id, finished, rx, coverage)
            })
        }
      };

      if let Some((relay_request_id, finished, mut rx, mut coverage)) = upstream_rx {
        let timeout = context.server_config.upstream_fetch_timeout;
        let mark_absent = |spans: Vec<Span>| {
          for (from, to) in spans {
            cache.mark_absent(from, to);
          }
        };
        // Whether the publisher delivered all it said it would.
        let mut complete = false;
        loop {
          tokio::select! {
            result = tokio::time::timeout(timeout, rx.recv()) => {
              match result {
                // The downstream End Location is already settled by the time a gap
                // is filled; the publisher's says how far its response reaches.
                Ok(Some(UpstreamFetchEvent::Accepted { end_location })) => {
                  trace!(
                    "Upstream FETCH {} for groups {}..{} accepted, end location {:?}",
                    relay_request_id, gap_start, gap_end, end_location
                  );
                  coverage.accepted(&end_location);
                }
                Ok(Some(UpstreamFetchEvent::EndOfRange { kind, location })) => {
                  mark_absent(coverage.marker(kind, &location));
                  let marker = FetchObject::EndOfRange {
                    kind,
                    group_id: location.group,
                    object_id: location.object,
                  };
                  write_entry(marker, &mut send_stream, &mut fetch_prev_ctx).await?;
                }
                Ok(Some(UpstreamFetchEvent::Object(object))) => {
                  // Cached here, into the track this FETCH holds: the track by that
                  // name may have been removed, or replaced, since it was asked for.
                  if let Ok(fetched) =
                    moqtail::model::data::object::Object::try_from_fetch(object.clone(), relay_track_id)
                  {
                    track.read().await.cache_fetched_object(&fetched).await;
                  }
                  mark_absent(
                    coverage.object(&Location::new(object.group_id, object.object_id)),
                  );
                  if !filter.matches(filter_fields(&object)) {
                    continue;
                  }
                  write_entry(
                    FetchObject::Object(object),
                    &mut send_stream,
                    &mut fetch_prev_ctx,
                  )
                  .await?;
                }
                Ok(Some(UpstreamFetchEvent::MalformedTrack)) => {
                  warn!(
                    "handle_fetch_messages | Malformed upstream FETCH for gap [{}, {}]",
                    gap_start, gap_end
                  );
                  let track_read = track.read().await;
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
                  complete = true;
                  finished.store(true, Ordering::Release);
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

        let delivered_all = complete && coverage.accepted
          || coverage.last.as_ref()
            == Some(&last_delivered(
              gap_start,
              gap_end,
              &start_location,
              &end_location,
              descending,
            ));
        if stop_reason == FetchStop::Running && !delivered_all {
          write_entry(
            unfilled(gap_start, gap_end),
            &mut send_stream,
            &mut fetch_prev_ctx,
          )
          .await?;
        }

        // A malformed response is not to be believed about anything.
        if stop_reason != FetchStop::MalformedTrack {
          mark_absent(coverage.finish(complete));
        }
      } else {
        write_entry(
          unfilled(gap_start, gap_end),
          &mut send_stream,
          &mut fetch_prev_ctx,
        )
        .await?;
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
  } else if send_stream.is_none() {
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
    FetchRangeError, concrete_end_location, delivered_start, empty_range_outcome, fill_groups,
    gap_range, last_delivered, local_state_answers, range_filter, resolve_fetch_range,
    unresolved_fetch_range,
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
  fn a_descending_fetch_across_groups_starts_its_start_group_at_object_0() {
    use moqtail::model::control::constant::GroupOrder;
    let start = || loc(5, 3);
    assert_eq!(
      delivered_start(start(), &loc(9, 2), GroupOrder::Descending),
      loc(5, 0)
    );
    assert_eq!(
      delivered_start(start(), &loc(5, 8), GroupOrder::Descending),
      loc(5, 3)
    );
    assert_eq!(
      delivered_start(start(), &loc(9, 2), GroupOrder::Ascending),
      loc(5, 3)
    );
  }

  #[test]
  fn only_a_fetch_with_an_end_claims_its_groups() {
    assert_eq!(fill_groups(&range(loc(5, 3), loc(9, 2))), Some((5, 9)));
    let open_ended = LocationFilter::AbsoluteStart { start: loc(5, 3) };
    assert_eq!(fill_groups(&open_ended), None);
    assert_eq!(fill_groups(&LocationFilter::None), None);
  }

  #[test]
  fn an_unfilled_gap_is_marked_up_to_the_last_location_it_delivers() {
    let (start, end) = (loc(5, 3), loc(9, 2));
    // Ascending: the end of the highest group, clipped to the request.
    assert_eq!(last_delivered(5, 7, &start, &end, false), loc(7, u64::MAX));
    assert_eq!(last_delivered(8, 9, &start, &end, false), loc(9, 2));
    // Descending: the end of the lowest group, which comes last.
    assert_eq!(last_delivered(5, 7, &start, &end, true), loc(5, u64::MAX));
    assert_eq!(last_delivered(9, 9, &start, &end, true), loc(9, 2));
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
    let gap = |first, last| gap_range(first, last, &start, &end);

    // The request's first group starts at its Start Location.
    assert_eq!(gap(5, 5), (loc(5, 3), loc(5, u64::MAX)));
    // A group inside the request is whole.
    assert_eq!(gap(7, 7), (loc(7, 0), loc(7, u64::MAX)));
    // The request's last group stops at its End Location.
    assert_eq!(gap(9, 9), (loc(9, 0), loc(9, 2)));
    // A gap across the whole request is the request.
    assert_eq!(gap(5, 9), (loc(5, 3), loc(9, 2)));
  }

  #[test]
  fn a_range_ending_with_its_last_group_is_asked_for_by_group() {
    assert_eq!(
      range_filter(&loc(7, 0), &loc(9, u64::MAX)),
      LocationFilter::AbsoluteStartGroupEnd {
        start: loc(7, 0),
        end_group: 9,
      }
    );
    assert_eq!(
      range_filter(&loc(9, 0), &loc(9, 2)),
      range(loc(9, 0), loc(9, 2))
    );
  }

  mod coverage {
    use super::super::{EndOfRangeKind, UpstreamCoverage};
    use super::loc;
    use moqtail::model::control::constant::GroupOrder;

    const MAX: u64 = u64::MAX;

    #[test]
    fn what_an_ascending_response_passes_over_is_absent() {
      let mut c = UpstreamCoverage::new(loc(1, 2), loc(3, 4), GroupOrder::Ascending);
      c.accepted(&loc(3, 4));
      // From the start up to the first Object.
      assert_eq!(c.object(&loc(1, 5)), [(loc(1, 2), loc(1, 4))]);
      // Consecutive Objects leave nothing between them.
      assert_eq!(c.object(&loc(1, 6)), []);
      // The rest of group 1 and all of group 2.
      assert_eq!(c.object(&loc(3, 0)), [(loc(1, 7), loc(2, MAX))]);
      // After the last Object, up to the End Location.
      assert_eq!(c.finish(true), [(loc(3, 1), loc(3, 4))]);
    }

    #[test]
    fn the_end_of_a_response_counts_only_after_fetch_ok_and_a_clean_close() {
      let mut cut = UpstreamCoverage::new(loc(0, 0), loc(2, MAX), GroupOrder::Ascending);
      cut.accepted(&loc(2, 5));
      cut.object(&loc(0, 0));
      assert_eq!(cut.finish(false), []);

      let mut unanswered = UpstreamCoverage::new(loc(0, 0), loc(2, MAX), GroupOrder::Ascending);
      unanswered.object(&loc(0, 0));
      assert_eq!(unanswered.finish(true), []);
    }

    #[test]
    fn the_publishers_end_location_bounds_what_is_absent() {
      let mut c = UpstreamCoverage::new(loc(0, 0), loc(5, MAX), GroupOrder::Ascending);
      // The Largest Object is {1,3}: anything after it may still be published.
      c.accepted(&loc(1, 3));
      c.object(&loc(1, 0));
      assert_eq!(c.finish(true), [(loc(1, 1), loc(1, 3))]);
    }

    #[test]
    fn an_empty_response_shows_the_whole_range_absent() {
      let mut c = UpstreamCoverage::new(loc(4, 1), loc(6, 2), GroupOrder::Ascending);
      c.accepted(&loc(6, 2));
      assert_eq!(c.finish(true), [(loc(4, 1), loc(6, 2))]);
    }

    #[test]
    fn only_a_non_existent_marker_shows_its_range_absent() {
      let mut c = UpstreamCoverage::new(loc(0, 0), loc(9, MAX), GroupOrder::Ascending);
      c.object(&loc(2, 0));
      assert_eq!(c.marker(EndOfRangeKind::Unknown, &loc(2, 5)), []);
      // Counted from after the unknown marker, not from {2,0}.
      assert_eq!(c.object(&loc(2, 7)), [(loc(2, 6), loc(2, 6))]);
      assert_eq!(c.marker(EndOfRangeKind::TimedOut, &loc(3, 1)), []);
      assert_eq!(
        c.marker(EndOfRangeKind::NonExistent, &loc(3, 4)),
        [(loc(3, 2), loc(3, 3)), (loc(3, 4), loc(3, 4))]
      );
    }

    #[test]
    fn a_descending_response_across_groups_covers_them_whole() {
      // Asked for {1,2}..{3,1}: groups 3, 2 and 1 come whole, highest first, with
      // group 3 only as far as the End Location.
      let mut c = UpstreamCoverage::new(loc(1, 2), loc(3, 1), GroupOrder::Descending);
      c.accepted(&loc(3, 1));
      assert_eq!(c.object(&loc(3, 1)), [(loc(3, 0), loc(3, 0))]);
      // Group 3 ends at the End Location, so nothing of it is left; group 2 is
      // skipped entirely, and group 1 starts at its object 0.
      assert_eq!(c.object(&loc(1, 0)), [(loc(2, 0), loc(2, MAX))]);
      assert_eq!(c.finish(true), [(loc(1, 1), loc(1, MAX))]);
    }

    #[test]
    fn a_descending_response_in_one_group_is_by_location() {
      let mut c = UpstreamCoverage::new(loc(4, 2), loc(4, 6), GroupOrder::Descending);
      c.accepted(&loc(4, 6));
      assert_eq!(c.object(&loc(4, 4)), [(loc(4, 2), loc(4, 3))]);
      assert_eq!(c.finish(true), [(loc(4, 5), loc(4, 6))]);
    }
  }
}
