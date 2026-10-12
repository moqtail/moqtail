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

//! Which subscription a received object belongs to.
//!
//! A publisher may give concurrent subscriptions to one track the same Track
//! Alias or different ones, so the alias alone cannot say. Each subscription's
//! filter is applied again to the object's location, and a publisher sends an
//! object once for every subscription it matches, so each subscription takes an
//! object once however many copies of it arrive.

use moqtail::model::common::location::Location;
use moqtail::model::control::constant::FilterType;
use moqtail::model::parameter::message_parameter::MessageParameter;
use std::collections::HashSet;

/// What a subscription delivers: from `start` on, up to and including
/// `end_group` when it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
  pub start: Location,
  pub end_group: Option<u64>,
}

impl Window {
  /// The window a SUBSCRIPTION_FILTER asks for, given the Largest Object the
  /// publisher reported in SUBSCRIBE_OK. A track with nothing published yet
  /// starts at the beginning.
  pub fn from_filter(filter: &MessageParameter, largest: Option<&Location>) -> Self {
    let MessageParameter::SubscriptionFilter {
      filter_type,
      start_location,
      end_group,
      relative_previous,
    } = filter
    else {
      return Self::everything();
    };
    let start = match filter_type {
      FilterType::LatestObject => largest.map_or(Location::new(0, 0), |l| {
        Location::new(l.group, l.object + 1)
      }),
      FilterType::NextGroupStart => {
        largest.map_or(Location::new(0, 0), |l| Location::new(l.group + 1, 0))
      }
      FilterType::AbsoluteStartFill | FilterType::AbsoluteRangeFill => {
        start_location.clone().unwrap_or_default()
      }
      FilterType::RelativeStartFill => largest.map_or(Location::new(0, 0), |l| {
        Location::new(l.group.saturating_sub(relative_previous.unwrap_or(0)), 0)
      }),
    };
    let end_group = match filter_type {
      FilterType::AbsoluteRangeFill => *end_group,
      _ => None,
    };
    Self { start, end_group }
  }

  pub fn everything() -> Self {
    Self {
      start: Location::new(0, 0),
      end_group: None,
    }
  }

  fn contains(&self, location: &Location) -> bool {
    *location >= self.start && self.end_group.is_none_or(|end| location.group <= end)
  }
}

struct Subscription {
  request_id: u64,
  label: String,
  track_alias: u64,
  window: Window,
  seen: HashSet<(u64, u64)>,
}

/// Where a received object went.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Route {
  /// The subscriptions taking it, by Request ID.
  pub owners: Vec<u64>,
  /// The subscriptions it belonged to that already had it.
  pub duplicates: usize,
}

#[derive(Default)]
pub struct Router {
  subscriptions: Vec<Subscription>,
}

impl Router {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn add(&mut self, request_id: u64, label: String, track_alias: u64, window: Window) {
    self.subscriptions.push(Subscription {
      request_id,
      label,
      track_alias,
      window,
      seen: HashSet::new(),
    });
  }

  pub fn label(&self, request_id: u64) -> &str {
    self
      .subscriptions
      .iter()
      .find(|s| s.request_id == request_id)
      .map_or("unknown", |s| s.label.as_str())
  }

  /// Hands an object to every subscription it belongs to that does not have it
  /// yet. Among the subscriptions holding its alias, those whose window covers it
  /// own it; when no window does -- an object the publisher sent from before a
  /// start this side could only estimate -- every one holding the alias does.
  pub fn route(&mut self, track_alias: u64, location: &Location) -> Route {
    let holders: Vec<usize> = (0..self.subscriptions.len())
      .filter(|&i| self.subscriptions[i].track_alias == track_alias)
      .collect();
    let covering: Vec<usize> = holders
      .iter()
      .copied()
      .filter(|&i| self.subscriptions[i].window.contains(location))
      .collect();
    let belongs_to = if covering.is_empty() {
      holders
    } else {
      covering
    };

    let mut route = Route::default();
    for i in belongs_to {
      let subscription = &mut self.subscriptions[i];
      if subscription.seen.insert((location.group, location.object)) {
        route.owners.push(subscription.request_id);
      } else {
        route.duplicates += 1;
      }
    }
    route
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn from(group: u64) -> Window {
    Window {
      start: Location::new(group, 0),
      end_group: None,
    }
  }

  fn range(start: u64, end: u64) -> Window {
    Window {
      start: Location::new(start, 0),
      end_group: Some(end),
    }
  }

  #[test]
  fn different_aliases_route_by_alias() {
    let mut router = Router::new();
    router.add(0, "a".into(), 10, from(0));
    router.add(2, "b".into(), 11, from(0));

    assert_eq!(router.route(10, &Location::new(1, 0)).owners, vec![0]);
    assert_eq!(router.route(11, &Location::new(1, 0)).owners, vec![2]);
  }

  #[test]
  fn a_shared_alias_routes_by_filter() {
    let mut router = Router::new();
    router.add(0, "early".into(), 7, range(0, 4));
    router.add(2, "late".into(), 7, from(5));

    assert_eq!(router.route(7, &Location::new(3, 1)).owners, vec![0]);
    assert_eq!(router.route(7, &Location::new(6, 0)).owners, vec![2]);
  }

  #[test]
  fn overlapping_filters_take_each_object_once() {
    let mut router = Router::new();
    router.add(0, "a".into(), 7, from(0));
    router.add(2, "b".into(), 7, from(0));

    // One copy per subscription arrives under the shared alias.
    let first = router.route(7, &Location::new(2, 0));
    assert_eq!(first.owners, vec![0, 2]);
    assert_eq!(first.duplicates, 0);

    let second = router.route(7, &Location::new(2, 0));
    assert!(second.owners.is_empty());
    assert_eq!(second.duplicates, 2);
  }

  #[test]
  fn an_object_no_window_covers_goes_to_the_alias_holders() {
    let mut router = Router::new();
    router.add(0, "a".into(), 7, from(5));

    assert_eq!(router.route(7, &Location::new(4, 3)).owners, vec![0]);
  }

  #[test]
  fn an_unknown_alias_belongs_to_no_one() {
    let mut router = Router::new();
    router.add(0, "a".into(), 7, from(0));

    assert_eq!(router.route(8, &Location::new(0, 0)), Route::default());
  }

  #[test]
  fn latest_object_starts_after_the_largest() {
    let filter = MessageParameter::new_subscription_filter(FilterType::LatestObject, None, None);
    let window = Window::from_filter(&filter, Some(&Location::new(3, 4)));
    assert_eq!(window.start, Location::new(3, 5));
    assert_eq!(window.end_group, None);
  }

  #[test]
  fn absolute_range_keeps_its_end() {
    let filter = MessageParameter::new_subscription_filter(
      FilterType::AbsoluteRangeFill,
      Some(Location::new(2, 0)),
      Some(5),
    );
    assert_eq!(Window::from_filter(&filter, None), range(2, 5));
  }
}
