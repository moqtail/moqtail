// Copyright 2026 The MOQtail Authors
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

//! Writing a FETCH's range onto its data stream, from the cache and from upstream.

use super::upstream::{handle_malformed_upstream_fetch, send_upstream_fetch_for_range};
use super::{FetchStop, PendingUpstreamFetch};
use crate::server::session::client::MOQTClient;
use crate::server::session::session_context::{SessionContext, UpstreamFetchEvent};
use crate::server::stream_id::{SendStreamKey, StreamId};
use crate::server::track::track_cache::FillClaim;
use core::result::Result::{Err, Ok};
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::data::constant::ObjectForwardingPreference;
use moqtail::model::data::fetch_header::FetchHeader;
use moqtail::model::data::fetch_object::{
  EndOfRangeKind, FetchObject, FetchObjectContext, FetchObjectPayload,
};
use moqtail::model::error::StreamResetCode;
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::combined_filter::{CombinedFilter, ObjectFields};
use moqtail::model::parameter::location_filter::LocationFilter;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::watch;
use tracing::{error, info, trace, warn};

/// The part of the groups `first..=last` to ask upstream for, for a request of
/// `start..=end`. Whatever comes back is forwarded as it arrives, so it must not reach
/// outside the request.
///
/// Every group the request covers is whole except its own first and last, so the
/// gap's whole groups are clipped to the request. For a request {5,3}..{9,2}: a gap of
/// group 5 is {5,3} to the end of group 5, a gap of group 7 is all of group 7, a gap
/// of group 9 is {9,0}..{9,2}, and a gap of all of them is the request itself.
fn requested_part(first: u64, last: u64, start: &Location, end: &Location) -> (Location, Location) {
  (
    Location::new(first, 0).max(start.clone()),
    Location::new(last, u64::MAX).min(end.clone()),
  )
}

/// The first Location a fetch stream for `start..=end` carries. A descending FETCH
/// across groups carries each group whole, the start group included: a gap before the
/// first Object of a group means those Objects do not exist, so leaving out the ones
/// before the Start Location would tell the subscriber they do not.
pub(crate) fn delivered_start(
  start: Location,
  end: &Location,
  group_order: GroupOrder,
) -> Location {
  if group_order == GroupOrder::Descending && start.group != end.group {
    Location::new(start.group, 0)
  } else {
    start
  }
}

/// The Object IDs of `group` a request of `start..=end` wants, clipped the same way.
fn objects_wanted(group: u64, start: &Location, end: &Location) -> (u64, u64) {
  let (lo, hi) = requested_part(group, group, start, end);
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
  requested_part(group, group, start, end).1
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
pub(crate) fn range_filter(start: &Location, end: &Location) -> LocationFilter {
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
struct ResponseAbsences {
  /// The first and last Locations the response can carry.
  start: Location,
  end: Location,
  descending: bool,
  /// Whether FETCH_OK has said where the response ends.
  fetch_ok: bool,
  /// The last Object or marker that arrived.
  last: Option<Location>,
}

impl ResponseAbsences {
  fn new(start: Location, end: Location, group_order: GroupOrder) -> Self {
    let descending = group_order == GroupOrder::Descending && start.group != end.group;
    let start = delivered_start(start, &end, group_order);
    Self {
      start,
      end,
      descending,
      fetch_ok: false,
      last: None,
    }
  }

  /// The publisher's End Location: the response may end short of what was asked for,
  /// at the Largest Object, whose group can still be growing.
  fn on_fetch_ok(&mut self, end_location: &Location) {
    if *end_location < self.end {
      self.end = end_location.clone();
    }
    self.fetch_ok = true;
  }

  fn on_object(&mut self, location: &Location) -> Vec<Span> {
    let spans = self.between(location);
    self.last = Some(location.clone());
    spans
  }

  /// A marker covers the Locations since the last entry, and its own.
  fn on_marker(&mut self, kind: EndOfRangeKind, location: &Location) -> Vec<Span> {
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
  fn on_end(self, closed_cleanly: bool) -> Vec<Span> {
    if !(closed_cleanly && self.fetch_ok) {
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
  pub fill_claim: Option<FillClaim>,
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
  track: Arc<crate::server::track::Track>,
  delivery: FetchDelivery,
  mut cancel_rx: watch::Receiver<FetchStop>,
) -> Result<bool, TerminationCode> {
  let (relay_track_id, cache, object_logger) = {
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
    fill_claim: _fill_claim,
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
  let mut last_waited_group: Option<u64> = None;
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
      .write_stream_object(&stream_id, object_id, vec![serialized], send_stream.clone())
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
        let cancellation = *cancel_rx.borrow();
        if cancellation != FetchStop::Running {
          stop_reason = cancellation;
          break;
        }
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
      let _claim = match &pending_upstream {
        Some(_) => None,
        None => match cache.claim_fill(gap_start, gap_end) {
          Ok(claim) => Some(claim),
          Err(in_flight) if last_waited_group != Some(group_id) => {
            last_waited_group = Some(group_id);
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
          let absences =
            ResponseAbsences::new(start_location.clone(), end_location.clone(), group_order);
          Some((
            pending.relay_request_id,
            pending.publisher,
            pending.closed_cleanly,
            pending.rx,
            absences,
          ))
        }
        None => {
          let (start, end) = requested_part(gap_start, gap_end, &start_location, &end_location);
          let gap = range_filter(&start, &end);
          send_upstream_fetch_for_range(&client, &context, &track, gap, group_order)
            .await
            .map(|(request_id, publisher, closed_cleanly, rx, _)| {
              let absences = ResponseAbsences::new(start, end, group_order);
              (request_id, publisher, closed_cleanly, rx, absences)
            })
        }
      };

      if let Some((relay_request_id, upstream_publisher, closed_cleanly, mut rx, mut absences)) =
        upstream_rx
      {
        let timeout = context.server_config.upstream_fetch_timeout;
        let mark_absent = |spans: Vec<Span>| {
          for (from, to) in spans {
            cache.mark_absent(from, to);
          }
        };
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
                  absences.on_fetch_ok(&end_location);
                }
                Ok(Some(UpstreamFetchEvent::EndOfRange { kind, location })) => {
                  mark_absent(absences.on_marker(kind, &location));
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
                    track.cache_fetched_object(&fetched).await;
                  }
                  mark_absent(
                    absences.on_object(&Location::new(object.group_id, object.object_id)),
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
                  handle_malformed_upstream_fetch(&context, &track, &upstream_publisher).await;
                  stop_reason = FetchStop::MalformedTrack;
                  break;
                }
                Ok(Some(UpstreamFetchEvent::StreamClosed)) => {
                  closed_cleanly.store(true, Ordering::Release);
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

        let cancellation = *cancel_rx.borrow();
        if cancellation != FetchStop::Running {
          stop_reason = cancellation;
        }
        let delivered_all = closed_cleanly.load(Ordering::Relaxed) && absences.fetch_ok
          || absences.last.as_ref()
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
          mark_absent(absences.on_end(closed_cleanly.load(Ordering::Relaxed)));
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
  use super::*;

  fn loc(group: u64, object: u64) -> Location {
    Location::new(group, object)
  }

  fn range(start: Location, end: Location) -> LocationFilter {
    LocationFilter::AbsoluteRange { start, end }
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
  fn a_gap_is_asked_for_only_as_far_as_the_request_reaches() {
    let (start, end) = (loc(5, 3), loc(9, 2));
    let gap = |first, last| requested_part(first, last, &start, &end);

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

  mod response_absences {
    use super::super::{EndOfRangeKind, ResponseAbsences};
    use moqtail::model::common::location::Location;

    fn loc(group: u64, object: u64) -> Location {
      Location::new(group, object)
    }
    use moqtail::model::control::constant::GroupOrder;

    const MAX: u64 = u64::MAX;

    #[test]
    fn what_an_ascending_response_passes_over_is_absent() {
      let mut c = ResponseAbsences::new(loc(1, 2), loc(3, 4), GroupOrder::Ascending);
      c.on_fetch_ok(&loc(3, 4));
      // From the start up to the first Object.
      assert_eq!(c.on_object(&loc(1, 5)), [(loc(1, 2), loc(1, 4))]);
      // Consecutive Objects leave nothing between them.
      assert_eq!(c.on_object(&loc(1, 6)), []);
      // The rest of group 1 and all of group 2.
      assert_eq!(c.on_object(&loc(3, 0)), [(loc(1, 7), loc(2, MAX))]);
      // After the last Object, up to the End Location.
      assert_eq!(c.on_end(true), [(loc(3, 1), loc(3, 4))]);
    }

    #[test]
    fn the_end_of_a_response_counts_only_after_fetch_ok_and_a_clean_close() {
      let mut cut = ResponseAbsences::new(loc(0, 0), loc(2, MAX), GroupOrder::Ascending);
      cut.on_fetch_ok(&loc(2, 5));
      cut.on_object(&loc(0, 0));
      assert_eq!(cut.on_end(false), []);

      let mut unanswered = ResponseAbsences::new(loc(0, 0), loc(2, MAX), GroupOrder::Ascending);
      unanswered.on_object(&loc(0, 0));
      assert_eq!(unanswered.on_end(true), []);
    }

    #[test]
    fn the_publishers_end_location_bounds_what_is_absent() {
      let mut c = ResponseAbsences::new(loc(0, 0), loc(5, MAX), GroupOrder::Ascending);
      // The Largest Object is {1,3}: anything after it may still be published.
      c.on_fetch_ok(&loc(1, 3));
      c.on_object(&loc(1, 0));
      assert_eq!(c.on_end(true), [(loc(1, 1), loc(1, 3))]);
    }

    #[test]
    fn an_empty_response_shows_the_whole_range_absent() {
      let mut c = ResponseAbsences::new(loc(4, 1), loc(6, 2), GroupOrder::Ascending);
      c.on_fetch_ok(&loc(6, 2));
      assert_eq!(c.on_end(true), [(loc(4, 1), loc(6, 2))]);
    }

    #[test]
    fn only_a_non_existent_marker_shows_its_range_absent() {
      let mut c = ResponseAbsences::new(loc(0, 0), loc(9, MAX), GroupOrder::Ascending);
      c.on_object(&loc(2, 0));
      assert_eq!(c.on_marker(EndOfRangeKind::Unknown, &loc(2, 5)), []);
      // Counted from after the unknown marker, not from {2,0}.
      assert_eq!(c.on_object(&loc(2, 7)), [(loc(2, 6), loc(2, 6))]);
      assert_eq!(c.on_marker(EndOfRangeKind::TimedOut, &loc(3, 1)), []);
      assert_eq!(
        c.on_marker(EndOfRangeKind::NonExistent, &loc(3, 4)),
        [(loc(3, 2), loc(3, 3)), (loc(3, 4), loc(3, 4))]
      );
    }

    #[test]
    fn a_descending_response_across_groups_covers_them_whole() {
      // Asked for {1,2}..{3,1}: groups 3, 2 and 1 come whole, highest first, with
      // group 3 only as far as the End Location.
      let mut c = ResponseAbsences::new(loc(1, 2), loc(3, 1), GroupOrder::Descending);
      c.on_fetch_ok(&loc(3, 1));
      assert_eq!(c.on_object(&loc(3, 1)), [(loc(3, 0), loc(3, 0))]);
      // Group 3 ends at the End Location, so nothing of it is left; group 2 is
      // skipped entirely, and group 1 starts at its object 0.
      assert_eq!(c.on_object(&loc(1, 0)), [(loc(2, 0), loc(2, MAX))]);
      assert_eq!(c.on_end(true), [(loc(1, 1), loc(1, MAX))]);
    }

    #[test]
    fn a_descending_response_in_one_group_is_by_location() {
      let mut c = ResponseAbsences::new(loc(4, 2), loc(4, 6), GroupOrder::Descending);
      c.on_fetch_ok(&loc(4, 6));
      assert_eq!(c.on_object(&loc(4, 4)), [(loc(4, 2), loc(4, 3))]);
      assert_eq!(c.on_end(true), [(loc(4, 5), loc(4, 6))]);
    }
  }
}
