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
use moqtail::model::parameter::location_filter::LocationFilter;
use moqtail::model::parameter::message_parameter::MessageParameter;
use std::sync::Arc;
use tokio::sync::{RwLock, watch};
use tracing::info;

/// The Objects a fill fetch stream carries: what `filter` selects under the
/// rules for a Fetch, which never reach past the Largest Object.
///
/// `None` when there is nothing to fill — no Objects published yet, or a Start
/// Location already past the Largest Object — in which case no stream is opened
/// and the subscription runs live-only.
///
/// Both Locations are inclusive. A filter that ends with a whole group has
/// `u64::MAX` as its last Object, which means all of it.
pub(crate) fn fill_fetch_range(
  filter: &LocationFilter,
  largest: Option<Location>,
) -> Option<(Location, Location)> {
  filter.fetch_range(largest)
}

/// The FILL_PARAMETERS in `parameters`, whose presence is what asks for a fill.
fn fill_parameters(parameters: &[MessageParameter]) -> Option<&[MessageParameter]> {
  match parameters
    .iter()
    .find(|p| p.type_value() == MessageParameterType::FillParameters as u64)
  {
    Some(MessageParameter::FillParameters { parameters }) => Some(parameters),
    _ => None,
  }
}

/// The range a fill covers: the Location Filter inside FILL_PARAMETERS, else the
/// subscription's own.
fn fill_filter(fill: &[MessageParameter], subscription_filter: &LocationFilter) -> LocationFilter {
  fill
    .iter()
    .find_map(|p| match p {
      MessageParameter::LocationFilter { filter } => Some(filter.clone()),
      _ => None,
    })
    .unwrap_or_else(|| subscription_filter.clone())
}

/// The subscriber priority and group order the fill fetch stream runs at: the
/// subscription's own, unless FILL_PARAMETERS overrides them.
fn fill_overrides(fill: &[MessageParameter], priority: u8, order: GroupOrder) -> (u8, GroupOrder) {
  let mut overridden = (priority, order);
  for param in fill {
    match param {
      MessageParameter::SubscriberPriority { priority } => overridden.0 = *priority,
      MessageParameter::GroupOrder { order } => overridden.1 = *order,
      _ => {}
    }
  }
  overridden
}

/// Opens the fill fetch stream FILL_PARAMETERS asks for, if it has anything to
/// carry. The stream is named by `request_id` — the SUBSCRIBE or REQUEST_UPDATE
/// that asked for the fill — and is counted towards the subscription's
/// PUBLISH_DONE Stream Count.
pub(crate) async fn open_fill_fetch_stream(
  context: Arc<SessionContext>,
  track: Arc<RwLock<Track>>,
  subscription: Arc<RwLock<Subscription>>,
  request_id: u64,
) {
  let (client, filter, range_filter, priority, group_order, forwarding) = {
    let sub = subscription.read().await;
    let state = sub.subscription_state.read().await;
    // A subscription with no FILL_PARAMETERS opens no fill fetch stream.
    let Some(fill) = fill_parameters(&state.subscribe_parameters) else {
      return;
    };
    let (priority, group_order) =
      fill_overrides(fill, state.subscriber_priority, state.group_order);
    (
      sub.subscriber(),
      fill_filter(fill, &state.location_filter),
      crate::server::filter_constructor::fill_range_filter(&state.subscribe_parameters, fill),
      priority,
      group_order,
      state.forward,
    )
  };

  // FILL_PARAMETERS carried while Forward State is 0 opens no fill fetch stream.
  if !forwarding {
    return;
  }

  let largest = track.read().await.largest_object().await;
  let Some((start, end)) = fill_fetch_range(&filter, largest) else {
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
    filter: range_filter,
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

  #[test]
  fn nothing_published_fills_nothing() {
    assert_eq!(fill_fetch_range(&LocationFilter::None, None), None);
    assert_eq!(
      fill_fetch_range(&LocationFilter::AbsoluteStart { start: loc(0, 0) }, None),
      None
    );
  }

  #[test]
  fn no_filter_fills_the_whole_track() {
    assert_eq!(
      fill_fetch_range(&LocationFilter::None, Some(loc(5, 3))),
      Some((loc(0, 0), loc(5, 3)))
    );
  }

  #[test]
  fn absolute_start_runs_to_the_largest_object() {
    assert_eq!(
      fill_fetch_range(
        &LocationFilter::AbsoluteStart { start: loc(2, 0) },
        Some(loc(5, 3))
      ),
      Some((loc(2, 0), loc(5, 3)))
    );
  }

  #[test]
  fn a_start_past_the_largest_object_fills_nothing() {
    assert_eq!(
      fill_fetch_range(
        &LocationFilter::AbsoluteStart { start: loc(9, 0) },
        Some(loc(5, 3))
      ),
      None
    );
  }

  #[test]
  fn next_object_fills_nothing() {
    assert_eq!(
      fill_fetch_range(&LocationFilter::NextObject, Some(loc(5, 3))),
      None
    );
  }

  #[test]
  fn an_end_group_before_the_largest_object_ends_the_fill_early() {
    assert_eq!(
      fill_fetch_range(
        &LocationFilter::AbsoluteStartGroupEnd {
          start: loc(2, 0),
          end_group: 4,
        },
        Some(loc(9, 1))
      ),
      Some((loc(2, 0), loc(4, u64::MAX)))
    );
  }

  #[test]
  fn an_end_object_is_the_last_one_filled() {
    assert_eq!(
      fill_fetch_range(
        &LocationFilter::AbsoluteRange {
          start: loc(2, 0),
          end: loc(4, 6),
        },
        Some(loc(9, 1))
      ),
      Some((loc(2, 0), loc(4, 6)))
    );
  }

  #[test]
  fn an_end_group_beyond_the_largest_object_does_not_extend_the_fill() {
    assert_eq!(
      fill_fetch_range(
        &LocationFilter::AbsoluteStartGroupEnd {
          start: loc(2, 0),
          end_group: 20,
        },
        Some(loc(9, 1))
      ),
      Some((loc(2, 0), loc(9, 1)))
    );
  }

  #[test]
  fn relative_start_counts_back_from_the_next_group() {
    // StartGroup 3 is two groups before the current one.
    assert_eq!(
      fill_fetch_range(
        &LocationFilter::RelativeStart { start_group: 3 },
        Some(loc(9, 1))
      ),
      Some((loc(7, 0), loc(9, 1)))
    );
  }

  #[test]
  fn relative_start_of_one_fills_the_current_group() {
    assert_eq!(
      fill_fetch_range(
        &LocationFilter::RelativeStart { start_group: 1 },
        Some(loc(9, 1))
      ),
      Some((loc(9, 0), loc(9, 1)))
    );
  }

  #[test]
  fn relative_start_of_zero_fills_nothing() {
    // The Next Group has nothing published yet.
    assert_eq!(
      fill_fetch_range(
        &LocationFilter::RelativeStart { start_group: 0 },
        Some(loc(9, 1))
      ),
      None
    );
  }

  #[test]
  fn relative_start_past_the_first_group_starts_at_zero() {
    assert_eq!(
      fill_fetch_range(
        &LocationFilter::RelativeStart { start_group: 50 },
        Some(loc(3, 1))
      ),
      Some((loc(0, 0), loc(3, 1)))
    );
  }

  #[test]
  fn no_fill_parameters_asks_for_no_fill() {
    assert!(
      fill_parameters(&[MessageParameter::new_location_filter(
        LocationFilter::AbsoluteStart { start: loc(0, 0) }
      )])
      .is_none()
    );
    assert!(fill_parameters(&[MessageParameter::new_fill_parameters(vec![])]).is_some());
  }

  #[test]
  fn a_filter_inside_fill_parameters_overrides_the_subscriptions() {
    let inner = LocationFilter::RelativeStart { start_group: 1 };
    assert_eq!(
      fill_filter(
        &[MessageParameter::new_location_filter(inner.clone())],
        &LocationFilter::NextObject
      ),
      inner
    );
    assert_eq!(
      fill_filter(&[], &LocationFilter::NextObject),
      LocationFilter::NextObject
    );
  }
}
