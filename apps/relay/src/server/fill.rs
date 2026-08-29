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
use crate::server::subscription::{
  DEFAULT_PUBLISHER_PRIORITY, Subscription, compute_stream_priority,
};
use crate::server::track::Track;
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::{FilterType, GroupOrder};
use moqtail::model::parameter::constant::MessageParameterType;
use moqtail::model::parameter::message_parameter::MessageParameter;
use std::sync::Arc;
use tokio::sync::{RwLock, watch};
use tracing::info;

/// The Objects a fill fetch stream carries: everything from the filter's Start
/// Location up to the fill boundary, which is the Largest Object or the End Group
/// when one is set and it ends earlier.
///
/// `None` when there is nothing to fill — no Objects published yet, or a Start
/// Location already past the Largest Object — in which case no stream is opened
/// and the subscription runs live-only.
///
/// The End Location returned is the last Object plus one, the encoding
/// [`serve_fetch_stream`] expects.
pub(crate) fn fill_fetch_range(
  filter_type: FilterType,
  start_location: Option<&Location>,
  end_group: Option<u64>,
  relative_previous: Option<u64>,
  largest: Option<Location>,
) -> Option<(Location, Location)> {
  if !filter_type.is_fetch_fill() {
    return None;
  }
  let largest = largest?;

  let start = match filter_type {
    FilterType::RelativeStartFill => {
      let back = relative_previous.unwrap_or(0);
      Location::new(largest.group.saturating_sub(back), 0)
    }
    _ => start_location.cloned().unwrap_or(Location::new(0, 0)),
  };

  if start > largest {
    return None;
  }

  // u64::MAX for the Object means the whole of that group: an End Group names a
  // group, not an Object within it.
  let end = match end_group {
    Some(eg) if eg < largest.group => Location::new(eg, u64::MAX),
    _ => Location::new(largest.group, largest.object + 1),
  };

  Some((start, end))
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
  let (
    client,
    filter_type,
    start_location,
    end_group,
    relative_previous,
    priority,
    group_order,
    forwarding,
  ) = {
    let sub = subscription.read().await;
    let state = sub.subscription_state.read().await;
    let (priority, group_order) = fill_overrides(
      &state.subscribe_parameters,
      state.subscriber_priority,
      state.group_order,
    );
    (
      sub.subscriber(),
      state.filter_type,
      state.start_location.clone(),
      if state.end_group > 0 {
        Some(state.end_group)
      } else {
        None
      },
      state.relative_previous,
      priority,
      group_order,
      state.forward,
    )
  };

  // A fill filter specified while Forward State is 0 opens no fill fetch stream.
  if !forwarding {
    return;
  }

  let largest = track.read().await.largest_object().await;
  let Some((start, end)) = fill_fetch_range(
    filter_type,
    start_location.as_ref(),
    end_group,
    relative_previous,
    largest,
  ) else {
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
    let _ = serve_fetch_stream(client, context, track, delivery, cancel_rx).await;
    let sub = subscription_for_task.read().await;
    sub.note_fill_stream_opened();
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
  fn a_non_fill_filter_fills_nothing() {
    assert_eq!(
      fill_fetch_range(FilterType::LatestObject, None, None, None, Some(loc(5, 2))),
      None
    );
  }

  #[test]
  fn nothing_published_fills_nothing() {
    assert_eq!(
      fill_fetch_range(
        FilterType::AbsoluteStartFill,
        Some(&loc(0, 0)),
        None,
        None,
        None
      ),
      None
    );
  }

  #[test]
  fn absolute_start_fill_runs_to_the_largest_object() {
    assert_eq!(
      fill_fetch_range(
        FilterType::AbsoluteStartFill,
        Some(&loc(2, 0)),
        None,
        None,
        Some(loc(5, 3))
      ),
      Some((loc(2, 0), loc(5, 4)))
    );
  }

  #[test]
  fn a_start_past_the_largest_object_fills_nothing() {
    assert_eq!(
      fill_fetch_range(
        FilterType::AbsoluteStartFill,
        Some(&loc(9, 0)),
        None,
        None,
        Some(loc(5, 3))
      ),
      None
    );
  }

  #[test]
  fn an_end_group_before_the_largest_object_ends_the_fill_early() {
    assert_eq!(
      fill_fetch_range(
        FilterType::AbsoluteRangeFill,
        Some(&loc(2, 0)),
        Some(4),
        None,
        Some(loc(9, 1))
      ),
      Some((loc(2, 0), loc(4, u64::MAX)))
    );
  }

  #[test]
  fn an_end_group_beyond_the_largest_object_does_not_extend_the_fill() {
    assert_eq!(
      fill_fetch_range(
        FilterType::AbsoluteRangeFill,
        Some(&loc(2, 0)),
        Some(20),
        None,
        Some(loc(9, 1))
      ),
      Some((loc(2, 0), loc(9, 2)))
    );
  }

  #[test]
  fn relative_start_fill_counts_back_from_the_largest_group() {
    assert_eq!(
      fill_fetch_range(
        FilterType::RelativeStartFill,
        None,
        None,
        Some(2),
        Some(loc(9, 1))
      ),
      Some((loc(7, 0), loc(9, 2)))
    );
  }

  #[test]
  fn relative_start_fill_of_zero_starts_at_the_current_group() {
    assert_eq!(
      fill_fetch_range(
        FilterType::RelativeStartFill,
        None,
        None,
        Some(0),
        Some(loc(9, 1))
      ),
      Some((loc(9, 0), loc(9, 2)))
    );
  }

  #[test]
  fn relative_start_fill_past_the_first_group_starts_at_zero() {
    assert_eq!(
      fill_fetch_range(
        FilterType::RelativeStartFill,
        None,
        None,
        Some(50),
        Some(loc(3, 1))
      ),
      Some((loc(0, 0), loc(3, 2)))
    );
  }
}
