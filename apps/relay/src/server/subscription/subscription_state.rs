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

//! What a subscription has asked for and how far it has got: plain data, no I/O.

use crate::server::session::client::switch_context::SwitchActivation;
use crate::server::subscription::SubscriptionOrigin;
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::parameter::combined_filter::CombinedFilter;
use moqtail::model::parameter::location_filter::LocationFilter;
use moqtail::model::parameter::message_parameter::MessageParameter;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SubscriptionState {
  pub subscriber_priority: u8,
  pub group_order: GroupOrder,
  pub forward: bool,
  pub location_filter: LocationFilter,
  /// Where `location_filter` starts, once resolved against the track.
  pub start_location: Option<Location>,
  /// The last group delivered, `None` when open-ended.
  pub end_group: Option<u64>,
  /// The last Object of `end_group` delivered, `None` for the whole group.
  pub end_object: Option<u64>,
  /// Set on a subscription a soft switch is draining: it keeps delivering up to
  /// `end_group` rather than stopping where it stands.
  pub soft_suspended: bool,
  /// Whether reaching the end of that drain also sends PUBLISH_DONE.
  pub publish_done_at_end: bool,
  /// Set on a subscription a switch is suspending. Its boundary only takes
  /// effect once this reports the activating subscription has delivered, so a
  /// switch to a track that never publishes does not cut delivery off.
  pub switch_activated: Option<SwitchActivation>,
  pub subscribe_parameters: Vec<MessageParameter>,
  /// What the subscriber's Range Filters let through, from `subscribe_parameters`.
  pub range_filter: CombinedFilter,
  pub last_sent_max_location: Option<Location>,
  /// The group sent before the one `last_sent_max_location` names, so the distance
  /// between a track's groups can be read off the two. See [`Self::sent_group_gap`].
  pub prior_sent_group: Option<u64>,
  pub last_received_object_location: Option<Location>,
  /// The group received before the one `last_received_object_location` names. Its
  /// counterpart for a track that has yet to send anything to this subscriber.
  pub prior_received_group: Option<u64>,
}

impl SubscriptionState {
  /// How long an Object may wait to be sent. See [`clamp_delivery_timeout`].
  pub(crate) fn delivery_timeout(&self, relay_ms: u64) -> Option<Duration> {
    let asked = self.subscribe_parameters.iter().find_map(|p| match p {
      MessageParameter::ObjectDeliveryTimeout { timeout } => Some(*timeout),
      _ => None,
    });
    clamp_delivery_timeout(asked, relay_ms)
  }

  pub fn update_last_sent_max_location(&mut self, location: Location) {
    match &self.last_sent_max_location {
      Some(current_max) => {
        if location > *current_max {
          if location.group > current_max.group {
            self.prior_sent_group = Some(current_max.group);
          }
          self.last_sent_max_location = Some(location);
        }
      }
      None => {
        self.last_sent_max_location = Some(location);
      }
    }
  }

  /// How far apart this track's group ids run, measured from the last two groups
  /// it sent, or `None` before it has sent two.
  ///
  /// Group ids are not dense on every track: where several tracks of one content
  /// share an id space, a track whose groups are longer than the shortest carries
  /// only every second, fourth, twelfth id. The group it is sending therefore
  /// covers everything up to the next id it will use, and the only guide to where
  /// that is is how far apart the last two were.
  // TODO: Until switch trigger in subscription.rs solvedthis is dead code.
  #[allow(dead_code)]
  pub fn sent_group_gap(&self) -> Option<u64> {
    let last = self.last_sent_max_location.as_ref()?.group;
    let prior = self.prior_sent_group?;
    last.checked_sub(prior).filter(|gap| *gap > 0)
  }

  /// How far apart this track's group ids run, measured from the last two groups
  /// that reached the relay for it, or `None` before two have.
  ///
  /// The same distance [`Self::sent_group_gap`] reports, for a subscription that
  /// has not sent anything yet: a track being switched to has to resume at one of
  /// its own group ids, and this is what says where the next one is.
  // TODO: Until switch trigger in subscription.rs solvedthis is dead code.
  #[allow(dead_code)]
  pub fn received_group_gap(&self) -> Option<u64> {
    let last = self.last_received_object_location.as_ref()?.group;
    let prior = self.prior_received_group?;
    last.checked_sub(prior).filter(|gap| *gap > 0)
  }

  /// Whether a switch has given this subscription a boundary that is not in
  /// force yet, because the subscription taking over has still delivered
  /// nothing. Holding the boundary keeps this track running rather than leaving
  /// the subscriber with neither.
  pub fn awaiting_switch_activation(&self) -> bool {
    self
      .switch_activated
      .as_ref()
      .is_some_and(|activation| !activation.is_set())
  }

  /// Adopts `filter`, resolving where it starts against `largest`.
  pub fn set_location_filter(&mut self, filter: LocationFilter, largest: Option<Location>) {
    self.start_location = filter.start_location(largest);
    let end = filter.end_location();
    self.end_group = end.as_ref().map(|end| end.group);
    self.end_object = end
      .map(|end| end.object)
      .filter(|object| *object != u64::MAX);
    self.location_filter = filter;
  }

  /// Adopts the filter a subscription is created with. A live filter has no start
  /// until [`crate::server::subscription::Subscription::resolve_live_start`] measures it against the track, and
  /// until then the subscription delivers whatever arrives.
  fn set_initial_location_filter(&mut self, filter: LocationFilter) {
    self.set_location_filter(filter, None);
    if live_start_location(&self.location_filter, None).is_some() {
      self.start_location = None;
    }
  }

  /// Whether `location` lies beyond where this subscription stops delivering.
  pub fn is_past_end(&self, location: &Location) -> bool {
    self.end_group.is_some_and(|end_group| {
      location.group > end_group
        || (location.group == end_group
          && self
            .end_object
            .is_some_and(|object| location.object > object))
    })
  }

  pub fn update_last_received_object_location(&mut self, location: Location) {
    match &self.last_received_object_location {
      Some(current_max) => {
        if location > *current_max {
          if location.group > current_max.group {
            self.prior_received_group = Some(current_max.group);
          }
          self.last_received_object_location = Some(location);
        }
      }
      None => {
        self.last_received_object_location = Some(location);
      }
    }
  }
}

impl From<SubscriptionOrigin> for SubscriptionState {
  fn from(origin: SubscriptionOrigin) -> Self {
    match origin {
      SubscriptionOrigin::Subscribe(subscribe) => {
        let subscriber_priority = subscribe
          .subscribe_parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::SubscriberPriority { priority } = p {
              Some(*priority)
            } else {
              None
            }
          })
          .unwrap_or(128);

        let group_order = subscribe
          .subscribe_parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::GroupOrder { order } = p {
              Some(*order)
            } else {
              None
            }
          })
          .unwrap_or(GroupOrder::Ascending);

        let forward = subscribe
          .subscribe_parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::Forward { forward } = p {
              Some(*forward)
            } else {
              None
            }
          })
          .unwrap_or(true);

        let mut state = Self {
          subscriber_priority,
          group_order,
          forward,
          location_filter: LocationFilter::None,
          start_location: None,
          end_group: None,
          end_object: None,
          soft_suspended: false,
          publish_done_at_end: false,
          switch_activated: None,
          subscribe_parameters: Vec::new(),
          range_filter: CombinedFilter::default(),
          last_sent_max_location: None,
          prior_sent_group: None,
          last_received_object_location: None,
          prior_received_group: None,
        };
        // An omitted filter is no filter.
        state.set_initial_location_filter(location_filter_of(&subscribe.subscribe_parameters));
        state.subscribe_parameters = subscribe.subscribe_parameters;
        // The handler refused the SUBSCRIBE if its filters could not be applied.
        state.range_filter =
          CombinedFilter::for_objects(&state.subscribe_parameters).unwrap_or_default();
        state
      }
      SubscriptionOrigin::Publish(publish) => {
        let subscriber_priority = publish
          .parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::SubscriberPriority { priority } = p {
              Some(*priority)
            } else {
              None
            }
          })
          .unwrap_or(128);

        let group_order = publish
          .parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::GroupOrder { order } = p {
              Some(*order)
            } else {
              None
            }
          })
          .unwrap_or(GroupOrder::Ascending);

        let forward = publish
          .parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::Forward { forward } = p {
              Some(*forward)
            } else {
              None
            }
          })
          .unwrap_or(true);

        let mut state = Self {
          subscriber_priority,
          group_order,
          forward,
          location_filter: LocationFilter::None,
          start_location: None,
          end_group: None,
          end_object: None,
          soft_suspended: false,
          publish_done_at_end: false,
          switch_activated: None,
          subscribe_parameters: Vec::new(),
          range_filter: CombinedFilter::default(),
          last_sent_max_location: None,
          prior_sent_group: None,
          last_received_object_location: None,
          prior_received_group: None,
        };
        // An omitted filter is no filter.
        state.set_initial_location_filter(location_filter_of(&publish.parameters));
        state.subscribe_parameters = publish.parameters;
        state
      }
    }
  }
}

/// The Location Filter in `params`, or no filter when it carries none.
pub(crate) fn location_filter_of(params: &[MessageParameter]) -> LocationFilter {
  params
    .iter()
    .find_map(|p| match p {
      MessageParameter::LocationFilter { filter } => Some(filter.clone()),
      _ => None,
    })
    .unwrap_or(LocationFilter::None)
}

/// The Start Location a live filter resolves to, or `None` where the filter
/// carries its own start, or has none, and keeps it.
///
/// The live filters are stated relative to what the track has published
/// already: Next Object starts at the Object after the largest, Relative Start
/// some number of groups back from the group after it. A track that has
/// published nothing starts at `{0,0}` either way, which delivers whatever comes
/// next.
pub(crate) fn live_start_location(
  filter: &LocationFilter,
  largest: Option<Location>,
) -> Option<Location> {
  match filter {
    LocationFilter::NextObject | LocationFilter::RelativeStart { .. } => {
      filter.start_location(largest)
    }
    LocationFilter::None
    | LocationFilter::AbsoluteStart { .. }
    | LocationFilter::AbsoluteStartGroupEnd { .. }
    | LocationFilter::AbsoluteRange { .. } => None,
  }
}

/// The subscriber's Object Delivery Timeout, capped by the relay's: a subscriber may
/// ask for stricter, never looser. 0 on either side means that side sets none.
fn clamp_delivery_timeout(asked_ms: Option<u64>, relay_ms: u64) -> Option<Duration> {
  let ms = match (asked_ms.filter(|ms| *ms > 0), relay_ms) {
    (Some(asked), 0) => asked,
    (Some(asked), relay) => asked.min(relay),
    (None, relay) => relay,
  };
  (ms > 0).then(|| Duration::from_millis(ms))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::server::subscription::DEFAULT_PUBLISHER_PRIORITY;

  #[test]
  fn a_subscriber_can_tighten_the_relays_delivery_timeout_but_not_loosen_it() {
    let ms = |ms| Some(Duration::from_millis(ms));
    assert_eq!(clamp_delivery_timeout(Some(600_000), 500), ms(500));
    assert_eq!(clamp_delivery_timeout(Some(200), 500), ms(200));
    assert_eq!(clamp_delivery_timeout(Some(200), 0), ms(200));
    assert_eq!(clamp_delivery_timeout(None, 500), ms(500));
    assert_eq!(clamp_delivery_timeout(Some(0), 500), ms(500));
    assert_eq!(clamp_delivery_timeout(None, 0), None);
  }

  #[test]
  fn next_object_starts_after_the_largest_object() {
    assert_eq!(
      live_start_location(&LocationFilter::NextObject, Some(Location::new(7, 3))),
      Some(Location::new(7, 4))
    );
  }

  #[test]
  fn a_relative_start_of_zero_starts_at_the_group_after_the_largest() {
    assert_eq!(
      live_start_location(
        &LocationFilter::RelativeStart { start_group: 0 },
        Some(Location::new(7, 3))
      ),
      Some(Location::new(8, 0))
    );
  }

  #[test]
  fn a_track_with_nothing_published_starts_at_the_beginning() {
    for filter in [
      LocationFilter::NextObject,
      LocationFilter::RelativeStart { start_group: 0 },
    ] {
      assert_eq!(
        live_start_location(&filter, None),
        Some(Location::new(0, 0)),
        "{filter:?} on an empty track"
      );
    }
  }

  fn sent(locations: &[(u64, u64)]) -> SubscriptionState {
    let mut state = SubscriptionState {
      location_filter: LocationFilter::None,
      start_location: None,
      end_group: None,
      end_object: None,
      forward: true,
      subscriber_priority: DEFAULT_PUBLISHER_PRIORITY,
      group_order: GroupOrder::Original,
      soft_suspended: false,
      publish_done_at_end: false,
      switch_activated: None,
      subscribe_parameters: vec![],
      range_filter: CombinedFilter::default(),
      last_sent_max_location: None,
      prior_sent_group: None,
      last_received_object_location: None,
      prior_received_group: None,
    };
    for (group, object) in locations {
      state.update_last_sent_max_location(Location::new(*group, *object));
      state.update_last_received_object_location(Location::new(*group, *object));
    }
    state
  }

  #[test]
  fn a_track_whose_group_ids_step_reports_the_step() {
    // Twelve ids per group: what it is sending covers everything up to the next.
    assert_eq!(
      sent(&[(288, 0), (288, 5), (300, 0)]).sent_group_gap(),
      Some(12)
    );
  }

  #[test]
  fn a_track_whose_group_ids_are_dense_reports_one() {
    assert_eq!(sent(&[(292, 0), (293, 0)]).sent_group_gap(), Some(1));
  }

  #[test]
  fn a_track_that_has_sent_one_group_cannot_say_how_far_the_next_is() {
    assert_eq!(sent(&[(292, 0), (292, 3)]).sent_group_gap(), None);
    assert_eq!(sent(&[]).sent_group_gap(), None);
  }

  #[test]
  fn a_track_reports_the_step_of_what_reached_it_before_it_sends() {
    assert_eq!(sent(&[(288, 0), (300, 0)]).received_group_gap(), Some(12));
    assert_eq!(sent(&[(288, 0), (288, 4)]).received_group_gap(), None);
  }

  #[test]
  fn an_absolute_filter_keeps_the_start_location_it_carries() {
    for filter in [
      LocationFilter::None,
      LocationFilter::AbsoluteStart {
        start: Location::new(2, 0),
      },
      LocationFilter::AbsoluteStartGroupEnd {
        start: Location::new(2, 0),
        end_group: 4,
      },
      LocationFilter::AbsoluteRange {
        start: Location::new(2, 0),
        end: Location::new(4, 1),
      },
    ] {
      assert_eq!(
        live_start_location(&filter, Some(Location::new(7, 3))),
        None,
        "{filter:?} states its own start, or none"
      );
    }
  }

  #[test]
  fn a_group_end_delivers_the_whole_of_its_last_group() {
    let mut state = sent(&[]);
    state.set_location_filter(
      LocationFilter::AbsoluteStartGroupEnd {
        start: Location::new(2, 0),
        end_group: 4,
      },
      None,
    );
    assert_eq!(state.start_location, Some(Location::new(2, 0)));
    assert!(!state.is_past_end(&Location::new(4, 1000)));
    assert!(state.is_past_end(&Location::new(5, 0)));
  }

  #[test]
  fn an_absolute_range_stops_after_its_end_object() {
    let mut state = sent(&[]);
    state.set_location_filter(
      LocationFilter::AbsoluteRange {
        start: Location::new(2, 0),
        end: Location::new(4, 6),
      },
      None,
    );
    assert!(!state.is_past_end(&Location::new(4, 6)));
    assert!(state.is_past_end(&Location::new(4, 7)));
    assert!(state.is_past_end(&Location::new(5, 0)));
  }

  #[test]
  fn a_filter_can_end_in_group_zero() {
    let mut state = sent(&[]);
    state.set_location_filter(
      LocationFilter::AbsoluteRange {
        start: Location::new(0, 0),
        end: Location::new(0, 5),
      },
      None,
    );
    assert!(!state.is_past_end(&Location::new(0, 5)));
    assert!(state.is_past_end(&Location::new(0, 6)));
    assert!(state.is_past_end(&Location::new(1, 0)));
  }

  #[test]
  fn a_live_filter_starts_unresolved() {
    let mut state = sent(&[]);
    state.set_initial_location_filter(LocationFilter::NextObject);
    assert_eq!(state.start_location, None);
    state.set_initial_location_filter(LocationFilter::AbsoluteStart {
      start: Location::new(3, 1),
    });
    assert_eq!(state.start_location, Some(Location::new(3, 1)));
  }

  #[test]
  fn an_open_ended_filter_has_no_end() {
    let mut state = sent(&[]);
    state.set_location_filter(LocationFilter::NextObject, Some(Location::new(7, 3)));
    assert_eq!(state.start_location, Some(Location::new(7, 4)));
    assert!(!state.is_past_end(&Location::new(u64::MAX, 0)));
  }

  #[test]
  fn no_filter_in_the_parameters_is_no_filter() {
    assert_eq!(location_filter_of(&[]), LocationFilter::None);
    assert_eq!(
      location_filter_of(&[MessageParameter::new_location_filter(
        LocationFilter::NextObject
      )]),
      LocationFilter::NextObject
    );
  }
}
