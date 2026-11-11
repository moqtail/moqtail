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

pub(crate) mod delivery;
mod upstream;

use crate::server::requests::DownstreamRequest;
use crate::server::session::client::MOQTClient;
use crate::server::session::session_context::{SessionContext, UpstreamFetchEvent};
use crate::server::track::{Track, TrackOrigin, TrackStatus};
use core::result::Result::{Err, Ok};
use delivery::{FetchDelivery, serve_fetch_stream};
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch_ok::FetchOk;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::location_filter::LocationFilter;
use moqtail::model::parameter::message_parameter::include_properties;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::FetchRequest;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};
use upstream::send_upstream_fetch_for_range;

pub(crate) const UPSTREAM_FETCH_CHANNEL_CAPACITY: usize = 64;

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
  track: &Arc<crate::server::track::Track>,
  group: u64,
) -> Option<u64> {
  let objects = track.cache.get_group(group).await?;
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
  pub closed_cleanly: Arc<AtomicBool>,
  pub rx: mpsc::Receiver<UpstreamFetchEvent>,
}

pub(crate) enum UpstreamFetchResponse {
  Accepted {
    end_location: Location,
    end_of_track: bool,
  },
  /// The publisher answered with REQUEST_ERROR.
  Rejected(Box<RequestError>),
  /// The request failed without an answer from the publisher.
  Failed(String),
  MalformedTrack,
}

/// What asking the publisher settled about a FETCH's range.
enum UpstreamRange {
  /// The publisher accepted, and its FETCH_OK gave the End Location and End Of Track.
  Resolved {
    pending: PendingUpstreamFetch,
    end_location: Location,
    end_of_track: bool,
  },
  /// The publisher refused the range. Its answer is authoritative.
  Rejected(Box<RequestError>),
  /// Nobody could say: no publisher to ask, no answer in time, or a REDIRECT, which
  /// the relay does not follow.
  Unanswered,
  MalformedTrack,
}

/// The relay's Request ID, whether the response arrived whole (set by its reader once
/// the fetch stream ends with a FIN), its Objects and markers, and its answer.
pub(crate) type UpstreamFetch = (
  u64,
  Arc<MOQTClient>,
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
fn groups_to_claim(filter: &LocationFilter) -> Option<(u64, u64)> {
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
  track: &Arc<crate::server::track::Track>,
  filter: LocationFilter,
  group_order: GroupOrder,
  largest: &mut Option<Location>,
) -> UpstreamRange {
  let sent = send_upstream_fetch_for_range(client, context, track, filter, group_order).await;
  let Some((relay_request_id, publisher, closed_cleanly, rx, response_rx)) = sent else {
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
    UpstreamFetchResponse::Accepted {
      end_location,
      end_of_track,
    } => {
      // The publisher will send up to its End Location, so the track reaches at least
      // that far.
      if largest.as_ref().is_none_or(|known| *known < end_location) {
        *largest = Some(end_location.clone());
      }
      UpstreamRange::Resolved {
        pending: PendingUpstreamFetch {
          relay_request_id,
          publisher,
          closed_cleanly,
          rx,
        },
        end_location,
        end_of_track,
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
    UpstreamFetchResponse::MalformedTrack => UpstreamRange::MalformedTrack,
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
      let filter = match crate::server::subscription::filter_constructor::object_filter(
        &fetch.parameters,
        context.server_config.max_filter_ranges,
      ) {
        Ok(filter) => filter,
        Err(e) => {
          info!("Rejecting FETCH {request_id}: {e}");
          let error = crate::server::subscription::filter_constructor::invalid_filter(&e);
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
                TrackOrigin::CacheOnly,
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
          track_alias: track.relay_track_id,
        };

        client
          .downstream_requests
          .insert(request_id, DownstreamRequest::Fetch(req))
          .await;
      }

      // Validate the requested range and clamp the FETCH_OK End Location to
      // published data. Set when the End Location had to be learned from the
      // publisher; the groups it covers are then served from it instead of being
      // fetched again.
      let mut pending_upstream: Option<PendingUpstreamFetch> = None;
      let mut fill_claim = None;
      // The publisher's End Location and whether it's the track's end, when it was asked.
      let mut upstream_end = None;
      let mut upstream_end_of_track = false;

      let (start_location, end_location) = {
        let mut largest = track.largest_object().await;

        // The relay answers on its own only when the requested end is one it has
        // already seen. Otherwise it may be behind the publisher, so it asks upstream
        // and takes the End Location from the publisher's FETCH_OK.
        let mut answered_locally = requested_end
          .as_ref()
          .is_some_and(|end| local_state_answers(&largest, end));
        // If another FETCH is already asking upstream for these groups, wait for it
        // once: what it brings in may let the relay answer this one itself.
        if !answered_locally
          && let (Some(end), Some((first, last))) =
            (&requested_end, groups_to_claim(&location_filter))
        {
          let cache = track.cache.clone();
          fill_claim = match cache.claim_fill(first, last) {
            Ok(claim) => Some(claim),
            Err(in_flight) => {
              let timeout = context.server_config.upstream_fetch_timeout;
              let _ = tokio::time::timeout(timeout, in_flight.lock()).await;
              largest = track.largest_object().await;
              answered_locally = local_state_answers(&largest, end);
              cache.claim_fill(first, last).ok()
            }
          };
        }
        let mut upstream_rejection = None;
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
              end_of_track,
            } => {
              pending_upstream = Some(pending);
              upstream_end = Some(end_location);
              upstream_end_of_track = end_of_track;
            }
            UpstreamRange::Rejected(err) => upstream_rejection = Some(err),
            UpstreamRange::Unanswered => {}
            UpstreamRange::MalformedTrack => {
              send_request_error(
                client.clone(),
                request_id,
                RequestErrorCode::MalformedTrack,
                ReasonPhrase::try_new("Malformed track".to_string()).unwrap(),
              )
              .await;
              return Ok(());
            }
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
        fill_claim = None;
      }

      let cancel_rx = client.downstream_requests.watch_fetch(request_id).await;
      if *cancel_rx.borrow() == FetchStop::MalformedTrack {
        send_request_error(
          client.clone(),
          request_id,
          RequestErrorCode::MalformedTrack,
          ReasonPhrase::try_new("Malformed track".to_string()).unwrap(),
        )
        .await;
        return Ok(());
      }

      // Send FetchOk on the request stream before delivering objects.
      {
        let track_properties = if include_properties(&fetch.parameters) {
          track.track_properties().await
        } else {
          Vec::new()
        };
        // End Of Track holds only while the response ends where the publisher's does.
        let end_of_track = upstream_end_of_track && upstream_end.as_ref() == Some(&end_location);
        let fetch_ok = FetchOk::new(end_of_track, end_location.clone(), vec![], track_properties);
        if let Err(error) = stream_handler
          .send(&ControlMessage::FetchOk(Box::new(fetch_ok)))
          .await
        {
          client.downstream_requests.remove(request_id).await;
          return Err(error);
        }
      }

      let delivery = FetchDelivery {
        request_id,
        start_location,
        end_location,
        group_order: fetch.group_order(),
        stream_priority: 0,
        pending_upstream,
        fill_claim,
        filter,
      };

      tokio::spawn(async move {
        let result = serve_fetch_stream(client.clone(), context, track, delivery, cancel_rx).await;
        client.downstream_requests.remove(request_id).await;
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
      client
        .downstream_requests
        .stop_fetch(target_request_id, FetchStop::UpdateFailed)
        .await;
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
  if client
    .downstream_requests
    .cancel_fetch(request_id)
    .await
    .is_some()
  {
    info!("Cancelled fetch delivery for request_id: {}", request_id);
  }
}

/// Whether a connected publisher could still serve Objects for this track, as
/// its origin or via an announced namespace. Used to decide whether an empty
/// local range is authoritative (INVALID_RANGE) or should be fetched upstream.
async fn has_upstream_publisher(
  context: &Arc<SessionContext>,
  track: &Arc<crate::server::track::Track>,
) -> bool {
  let full_track_name = track.full_track_name.clone();

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
  client.downstream_requests.remove(request_id).await;
}

#[cfg(test)]
mod tests {
  use super::{
    FetchRangeError, concrete_end_location, empty_range_outcome, groups_to_claim,
    local_state_answers, resolve_fetch_range, unresolved_fetch_range,
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
  fn only_a_fetch_with_an_end_claims_its_groups() {
    assert_eq!(groups_to_claim(&range(loc(5, 3), loc(9, 2))), Some((5, 9)));
    let open_ended = LocationFilter::AbsoluteStart { start: loc(5, 3) };
    assert_eq!(groups_to_claim(&open_ended), None);
    assert_eq!(groups_to_claim(&LocationFilter::None), None);
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
}
