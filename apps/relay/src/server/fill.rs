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

use crate::server::message_handlers::fetch_handler::{
  FetchDelivery, FetchStop, serve_fetch_stream,
};
use crate::server::session_context::SessionContext;
use crate::server::subscription::{DEFAULT_PUBLISHER_PRIORITY, Subscription};
use crate::server::track::Track;
use crate::server::utils::compute_stream_priority;
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::parameter::constant::MessageParameterType;
use moqtail::model::parameter::message_parameter::{MessageParameter, ResolvedLocationFilter};
use std::sync::Arc;
use tokio::sync::{RwLock, watch};
use tracing::info;

/// Whether this subscription asks for a fill fetch stream at all, and which
/// LOCATION_FILTER selects its range.
///
/// Draft-20 keys the fill on FILL_PARAMETERS being present (§5.1.3), where draft-18
/// keyed it on one of three fill Filter Types. The Filter Type is off the wire, and
/// the fill range is now stated independently of the subscription's own, so a
/// subscriber can fetch Groups before the live edge while the subscription itself
/// starts at the next Group -- which the old coupling could not express.
///
/// `Some(None)` is a fill over the whole track up to Largest Object: FILL_PARAMETERS
/// carries a zero-length filter, or neither it nor the subscription states one.
/// `None` is no fill fetch stream.
pub(crate) fn fill_range_filter(
  parameters: &[MessageParameter],
) -> Option<Option<MessageParameter>> {
  let Some(MessageParameter::FillParameters { parameters: fill }) = parameters
    .iter()
    .find(|p| matches!(p, MessageParameter::FillParameters { .. }))
  else {
    return None;
  };
  let is_filter = |p: &&MessageParameter| matches!(p, MessageParameter::LocationFilter { .. });
  // The filter inside FILL_PARAMETERS overrides the subscription's for the fill.
  let chosen = fill
    .iter()
    .find(is_filter)
    .or_else(|| parameters.iter().find(is_filter));
  Some(match chosen {
    // A zero-length filter is a real value meaning "no filter", so it reads as the
    // whole track rather than as an absent parameter.
    Some(f) if !f.location_filter_is_empty().unwrap_or(true) => Some(f.clone()),
    _ => None,
  })
}

/// The Objects a fill fetch stream carries: everything from the fill range's Start
/// Location up to the fill boundary, which is the Largest Object or the range's End
/// Group when one is set and it ends earlier.
///
/// The filter is evaluated under Fetch rules, so the range never extends beyond
/// Largest Object (§5.1.3). `filter` is `None` for the whole-track fill.
///
/// `None` when there is nothing to fill -- no Objects published yet, or a Start
/// Location already past the Largest Object -- in which case no stream is opened
/// and the subscription runs live-only.
///
/// The End Location returned is the last Object plus one, the encoding
/// [`serve_fetch_stream`] expects.
pub(crate) fn fill_fetch_range(
  filter: Option<&MessageParameter>,
  largest: Option<Location>,
) -> Option<(Location, Location)> {
  let largest = largest?;
  let whole_track = || ResolvedLocationFilter {
    start: Location::new(0, 0),
    end_group: None,
    end_object: None,
  };
  let resolved = match filter {
    Some(f) => f.resolve_location_filter(Some(largest.clone()))?,
    None => whole_track(),
  };

  if resolved.start > largest {
    return None;
  }

  let end = match resolved.end_group {
    // An End Group before the live edge ends the fill there. Without an End Object it
    // names a group rather than an Object within it, so the whole group is carried.
    Some(eg) if eg < largest.group => Location::new(
      eg,
      resolved
        .end_object
        .map_or(u64::MAX, |o| o.saturating_add(1)),
    ),
    _ => Location::new(largest.group, largest.object + 1),
  };

  Some((resolved.start, end))
}

/// The subscriber priority and group order the fill fetch stream runs at: the
/// subscription's own, unless FILL_PARAMETERS overrides them.
fn fill_overrides(
  parameters: &[MessageParameter],
  priority: u8,
  order: GroupOrder,
) -> (u8, GroupOrder) {
  let Some(MessageParameter::FillParameters { parameters }) = parameters
    .iter()
    .find(|p| p.type_value() == MessageParameterType::FillParameters as u64)
  else {
    return (priority, order);
  };
  let mut overridden = (priority, order);
  for param in parameters {
    match param {
      MessageParameter::SubscriberPriority { priority } => overridden.0 = *priority,
      MessageParameter::GroupOrder { order } => overridden.1 = *order,
      _ => {}
    }
  }
  overridden
}

/// Opens the fill fetch stream a fill filter type asks for, if it has anything to
/// carry. The stream is named by `request_id` — the SUBSCRIBE or REQUEST_UPDATE
/// that asked for the fill — and is counted towards the subscription's
/// PUBLISH_DONE Stream Count.
pub(crate) async fn open_fill_fetch_stream(
  context: Arc<SessionContext>,
  track: Arc<RwLock<Track>>,
  subscription: Arc<RwLock<Subscription>>,
  request_id: u64,
) {
  let (client, fill_filter, priority, group_order, forwarding) = {
    let sub = subscription.read().await;
    let state = sub.subscription_state.read().await;
    let (priority, group_order) = fill_overrides(
      &state.subscribe_parameters,
      state.subscriber_priority,
      state.group_order,
    );
    (
      sub.subscriber(),
      fill_range_filter(&state.subscribe_parameters),
      priority,
      group_order,
      state.forward,
    )
  };

  // No FILL_PARAMETERS, no fill fetch stream.
  let Some(fill_filter) = fill_filter else {
    return;
  };

  // FILL_PARAMETERS carried while Forward State is 0 opens no fill fetch stream.
  if !forwarding {
    return;
  }

  let largest = track.read().await.largest_object().await;
  let Some((start, end)) = fill_fetch_range(fill_filter.as_ref(), largest) else {
    info!(
      "fill: nothing to fill for request {} on subscriber {}; live only",
      request_id, client.connection_id
    );
    return;
  };

  info!(
    "fill: opening fill fetch stream for request {} on subscriber {} over {:?}..{:?}",
    request_id, client.connection_id, start, end
  );

  let (cancel_tx, cancel_rx) = watch::channel(FetchStop::Running);
  subscription
    .read()
    .await
    .register_fill_stream(request_id, cancel_tx)
    .await;

  let delivery = FetchDelivery {
    request_id,
    start_location: start.clone(),
    end_location: end,
    group_order,
    stream_priority: compute_stream_priority(
      priority,
      DEFAULT_PUBLISHER_PRIORITY,
      group_order,
      start.group,
    ),
    pending_upstream: None,
  };

  let subscription_for_task = subscription.clone();
  tokio::spawn(async move {
    let opened = serve_fetch_stream(client, context, track, delivery, cancel_rx).await;
    let sub = subscription_for_task.read().await;
    // Counted only if the subscriber actually got a stream: PUBLISH_DONE's Stream
    // Count is what the subscriber checks its own tally against, and a fill stopped
    // before its first object -- which is what suspending a switch does -- opens none.
    if opened.unwrap_or(false) {
      sub.note_fill_stream_opened();
    }
    sub.unregister_fill_stream(request_id).await;
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  fn loc(group: u64, object: u64) -> Location {
    Location::new(group, object)
  }

  fn absolute(start: Location) -> MessageParameter {
    MessageParameter::new_location_filter_absolute_start(start)
  }

  fn ranged(start: Location, end_group: u64) -> MessageParameter {
    MessageParameter::new_location_filter_absolute_range(start, end_group, None)
  }

  #[test]
  fn no_fill_parameters_means_no_fill_fetch_stream() {
    // Draft-20's trigger is FILL_PARAMETERS, not a filter type: a subscription that
    // states a Location filter but no FILL_PARAMETERS runs live-only.
    assert_eq!(fill_range_filter(&[absolute(loc(2, 0))]), None);
    assert_eq!(fill_range_filter(&[]), None);
  }

  #[test]
  fn fill_parameters_alone_fills_the_whole_track() {
    assert_eq!(
      fill_range_filter(&[MessageParameter::new_fill_parameters(vec![])]),
      Some(None)
    );
  }

  #[test]
  fn a_zero_length_filter_inside_fill_parameters_fills_the_whole_track() {
    let empty = MessageParameter::LocationFilter {
      start_group: None,
      start_object: None,
      end_group_delta: None,
      end_object: None,
    };
    assert_eq!(
      fill_range_filter(&[
        absolute(loc(2, 0)),
        MessageParameter::new_fill_parameters(vec![empty]),
      ]),
      Some(None),
      "the zero-length form removes the filter rather than falling back"
    );
  }

  #[test]
  fn the_subscriptions_filter_selects_the_range_when_fill_parameters_omits_one() {
    assert_eq!(
      fill_range_filter(&[
        absolute(loc(2, 0)),
        MessageParameter::new_fill_parameters(vec![]),
      ]),
      Some(Some(absolute(loc(2, 0))))
    );
  }

  #[test]
  fn a_filter_inside_fill_parameters_overrides_the_subscriptions() {
    // The case §5.1.3 calls out: fetch Groups before the live edge while the
    // subscription itself starts at the next Group.
    assert_eq!(
      fill_range_filter(&[
        MessageParameter::new_location_filter_next_group(),
        MessageParameter::new_fill_parameters(vec![absolute(loc(2, 0))]),
      ]),
      Some(Some(absolute(loc(2, 0))))
    );
  }

  #[test]
  fn nothing_published_fills_nothing() {
    assert_eq!(fill_fetch_range(Some(&absolute(loc(0, 0))), None), None);
    assert_eq!(fill_fetch_range(None, None), None);
  }

  #[test]
  fn a_whole_track_fill_runs_from_the_origin_to_the_largest_object() {
    assert_eq!(
      fill_fetch_range(None, Some(loc(5, 3))),
      Some((loc(0, 0), loc(5, 4)))
    );
  }

  #[test]
  fn an_absolute_start_runs_to_the_largest_object() {
    assert_eq!(
      fill_fetch_range(Some(&absolute(loc(2, 0))), Some(loc(5, 3))),
      Some((loc(2, 0), loc(5, 4)))
    );
  }

  #[test]
  fn a_start_past_the_largest_object_fills_nothing() {
    assert_eq!(
      fill_fetch_range(Some(&absolute(loc(9, 0))), Some(loc(5, 3))),
      None
    );
  }

  #[test]
  fn an_end_group_before_the_largest_object_ends_the_fill_early() {
    assert_eq!(
      fill_fetch_range(Some(&ranged(loc(2, 0), 4)), Some(loc(9, 1))),
      Some((loc(2, 0), loc(4, u64::MAX)))
    );
  }

  #[test]
  fn an_end_object_bounds_the_last_group_of_the_fill() {
    let bounded = MessageParameter::new_location_filter_absolute_range(loc(2, 0), 4, Some(7));
    assert_eq!(
      fill_fetch_range(Some(&bounded), Some(loc(9, 1))),
      // The range is inclusive at both ends and the End Location is last-plus-one.
      Some((loc(2, 0), loc(4, 8)))
    );
  }

  #[test]
  fn an_end_group_beyond_the_largest_object_does_not_extend_the_fill() {
    assert_eq!(
      fill_fetch_range(Some(&ranged(loc(2, 0), 20)), Some(loc(9, 1))),
      Some((loc(2, 0), loc(9, 2)))
    );
  }

  #[test]
  fn a_relative_filter_counts_back_from_the_largest_group() {
    assert_eq!(
      fill_fetch_range(
        Some(&MessageParameter::new_location_filter_relative_groups(2)),
        Some(loc(9, 1))
      ),
      Some((loc(7, 0), loc(9, 2)))
    );
  }

  #[test]
  fn a_relative_filter_of_zero_starts_at_the_current_group() {
    assert_eq!(
      fill_fetch_range(
        Some(&MessageParameter::new_location_filter_relative_groups(0)),
        Some(loc(9, 1))
      ),
      Some((loc(9, 0), loc(9, 2)))
    );
  }

  #[test]
  fn a_relative_filter_past_the_first_group_starts_at_zero() {
    assert_eq!(
      fill_fetch_range(
        Some(&MessageParameter::new_location_filter_relative_groups(50)),
        Some(loc(3, 1))
      ),
      Some((loc(0, 0), loc(3, 2)))
    );
  }
}
