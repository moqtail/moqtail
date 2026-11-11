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

//! The Range Filters a subscriber asks for, turned into the filter the relay applies.
//!
//! The relay applies them itself, per subscriber, on the way out. They are never sent
//! upstream: one upstream subscription serves every subscriber of a track, and letting
//! one subscriber's filters shape it would decide what the others can be served.
//!
//! A request is checked as a whole before anything is applied. Filters that cannot be
//! honoured -- more Ranges than the relay advertised, the same Type, SetID and Property
//! Type twice, an odd Property Type, a priority above 255, a Range past 2^64 - 1, or a
//! track filter where Objects are being chosen -- are refused with
//! REQUEST_ERROR INVALID_FILTER, never silently dropped.

use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::parameter::combined_filter::{CombinedFilter, check_filter_range_limit};
use moqtail::model::parameter::message_parameter::{
  MessageParameter, apply_message_parameter_update,
};
use moqtail::model::parameter::range_filter::{RangeFilterError, RangeFilterKind};
use moqtail::model::property::track_property::TrackProperty;
use std::collections::BTreeMap;

/// The REQUEST_ERROR for filters the relay cannot honour.
pub(crate) fn invalid_filter(error: &RangeFilterError) -> RequestError {
  let reason = ReasonPhrase::try_new(error.to_string())
    .unwrap_or_else(|_| ReasonPhrase::try_new("invalid filter".to_string()).unwrap());
  RequestError::new(RequestErrorCode::InvalidFilter, 0, reason)
}

/// Checks the filters a SUBSCRIBE or FETCH carries, those in its FILL_PARAMETERS
/// included, against a MAX_FILTER_RANGES of `limit`, and returns the filter its
/// Objects pass.
pub(crate) fn object_filter(
  params: &[MessageParameter],
  limit: u64,
) -> Result<CombinedFilter, RangeFilterError> {
  check_filter_range_limit(params, limit)?;
  for param in params {
    if let MessageParameter::FillParameters { parameters } = param {
      CombinedFilter::for_objects(parameters)?;
    }
  }
  CombinedFilter::for_objects(params)
}

/// Checks a REQUEST_UPDATE to a subscription whose parameters are `current`, and
/// returns the filter the subscription is left with.
///
/// The update is checked on its own, so that with a limit of 0 even a zero-length
/// removal is refused, and then for what it leaves: an update adding a kind adds to
/// the Ranges the subscription already holds.
pub(crate) fn updated_filter(
  current: &[MessageParameter],
  update: &[MessageParameter],
  limit: u64,
) -> Result<CombinedFilter, RangeFilterError> {
  check_filter_range_limit(update, limit)?;
  let mut resulting = current.to_vec();
  apply_message_parameter_update(&mut resulting, update.to_vec());
  object_filter(&resulting, limit)
}

/// Checks the filters a SUBSCRIBE_TRACKS carries, where track filters choose which
/// tracks are pushed and the rest filter the Objects of those that are. Those in its
/// FILL_PARAMETERS filter the fills of the pushed tracks.
pub(crate) fn tracks_filter(
  params: &[MessageParameter],
  limit: u64,
) -> Result<CombinedFilter, RangeFilterError> {
  check_filter_range_limit(params, limit)?;
  for param in params {
    if let MessageParameter::FillParameters { parameters } = param {
      CombinedFilter::for_objects(parameters)?;
    }
  }
  CombinedFilter::new(params)
}

/// Checks a REQUEST_UPDATE to a SUBSCRIBE_TRACKS whose parameters are `current`.
/// The filters it leaves apply to the tracks pushed from then on.
pub(crate) fn updated_tracks_filter(
  current: &[MessageParameter],
  update: &[MessageParameter],
  limit: u64,
) -> Result<CombinedFilter, RangeFilterError> {
  check_filter_range_limit(update, limit)?;
  let mut resulting = current.to_vec();
  apply_message_parameter_update(&mut resulting, update.to_vec());
  tracks_filter(&resulting, limit)
}

/// The Range Filters a track pushed for a SUBSCRIBE_TRACKS is subscribed with:
/// those of every SetID whose track filters its Track Properties pass, without the
/// track filters. `None` when no SetID passes, so the track is not to be pushed.
///
/// A passing SetID with no Object filters passes every Object, and SetIDs are
/// OR'd, so then the track is subscribed unfiltered.
pub(crate) fn filters_for_track(
  params: &[MessageParameter],
  track_properties: &[TrackProperty],
) -> Option<Vec<MessageParameter>> {
  let mut sets: BTreeMap<u8, Vec<&MessageParameter>> = BTreeMap::new();
  for param in params {
    if let MessageParameter::RangeFilter {
      filter: Some(filter),
      ..
    } = param
    {
      sets.entry(filter.set_id).or_default().push(param);
    }
  }
  if sets.is_empty() {
    return Some(Vec::new());
  }

  let is_track_filter = |p: &MessageParameter| {
    matches!(
      p,
      MessageParameter::RangeFilter {
        kind: RangeFilterKind::TrackProperty,
        ..
      }
    )
  };
  let mut kept = Vec::new();
  let mut any_passed = false;
  for filters in sets.values() {
    let track: Vec<MessageParameter> = filters
      .iter()
      .filter(|p| is_track_filter(p))
      .map(|p| (*p).clone())
      .collect();
    let passes = CombinedFilter::new(&track)
      .ok()
      .and_then(|net| net.narrow_for_track(track_properties))
      .is_some();
    if !passes {
      continue;
    }
    any_passed = true;
    let objects: Vec<MessageParameter> = filters
      .iter()
      .filter(|p| !is_track_filter(p))
      .map(|p| (*p).clone())
      .collect();
    if objects.is_empty() {
      return Some(Vec::new());
    }
    kept.extend(objects);
  }
  any_passed.then_some(kept)
}

/// The filter a fill fetch stream applies: the subscription's Range Filters, except
/// that a kind FILL_PARAMETERS carries replaces the subscription's filters of that
/// kind, and a zero-length one there removes them for the fill.
pub(crate) fn fill_range_filter(
  subscription: &[MessageParameter],
  fill: &[MessageParameter],
) -> CombinedFilter {
  let is_range_filter = |p: &&MessageParameter| matches!(p, MessageParameter::RangeFilter { .. });
  let mut params: Vec<_> = subscription
    .iter()
    .filter(is_range_filter)
    .cloned()
    .collect();
  apply_message_parameter_update(
    &mut params,
    fill.iter().filter(is_range_filter).cloned().collect(),
  );
  // Both were checked when the request carrying them arrived.
  CombinedFilter::for_objects(&params).unwrap_or_default()
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::parameter::range_filter::{Range, RangeFilter, RangeFilterKind};

  fn filter(kind: RangeFilterKind, set_id: u8, ranges: &[Range]) -> MessageParameter {
    let property_type = kind.has_property_type().then_some(0x3C);
    MessageParameter::new_range_filter(
      kind,
      RangeFilter::new(set_id, property_type, ranges).unwrap(),
    )
  }

  #[test]
  fn a_request_without_filters_passes_everything_whatever_the_limit() {
    let net = object_filter(&[MessageParameter::new_forward(true)], 0).unwrap();
    assert!(net.is_empty());
  }

  #[test]
  fn filters_within_the_limit_are_taken() {
    let params = [
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 1)]),
      filter(RangeFilterKind::Priority, 1, &[Range::new(0, 63)]),
    ];
    let net = object_filter(&params, 2).unwrap();
    assert_eq!(net.range_count(), 2);
  }

  #[test]
  fn filters_past_the_limit_are_refused() {
    let params = [filter(
      RangeFilterKind::ObjectId,
      0,
      &[Range::new(0, 1), Range::new(5, 6)],
    )];
    assert!(matches!(
      object_filter(&params, 1),
      Err(RangeFilterError::TooManyRanges { count: 2, limit: 1 })
    ));
    assert!(object_filter(&params, 0).is_err());
  }

  #[test]
  fn filters_that_cannot_be_applied_are_refused() {
    let duplicate = [
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 0)]),
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(4, 4)]),
    ];
    let track = [filter(
      RangeFilterKind::TrackProperty,
      0,
      &[Range::new(1, 1)],
    )];
    let priority = [filter(RangeFilterKind::Priority, 0, &[Range::new(0, 300)])];
    for params in [&duplicate[..], &track[..], &priority[..]] {
      assert!(object_filter(params, 8).is_err(), "{params:?}");
    }
  }

  #[test]
  fn filters_inside_fill_parameters_are_checked_too() {
    let duplicate = MessageParameter::new_fill_parameters(vec![
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 0)]),
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(4, 4)]),
    ]);
    assert!(object_filter(&[duplicate], 8).is_err());
  }

  #[test]
  fn an_update_is_checked_for_what_it_leaves() {
    let current = [filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 1)])];
    // Another kind adds to the Range already held.
    let adding = [filter(RangeFilterKind::Subgroup, 0, &[Range::new(0, 0)])];
    assert!(updated_filter(&current, &adding, 1).is_err());
    assert_eq!(
      updated_filter(&current, &adding, 2).unwrap().range_count(),
      2
    );
    // Replacing the kind held does not.
    let replacing = [filter(RangeFilterKind::ObjectId, 0, &[Range::new(4, 4)])];
    assert_eq!(
      updated_filter(&current, &replacing, 1)
        .unwrap()
        .range_count(),
      1
    );
  }

  #[test]
  fn a_fill_keeps_the_subscriptions_filters_except_the_kinds_it_carries() {
    let subscription = [
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 0)]),
      filter(RangeFilterKind::Priority, 0, &[Range::new(0, 63)]),
    ];
    let fill = [filter(RangeFilterKind::ObjectId, 0, &[Range::new(4, 9)])];
    let net = fill_range_filter(&subscription, &fill);
    let fields = |object, priority| moqtail::model::parameter::combined_filter::ObjectFields {
      subgroup_id: Some(0),
      object_id: object,
      publisher_priority: priority,
      properties: None,
    };
    // Object ID from the fill, priority from the subscription.
    assert!(net.matches(fields(5, 10)));
    assert!(!net.matches(fields(0, 10)));
    assert!(!net.matches(fields(5, 200)));

    // A zero-length filter in the fill removes the kind for the fill alone.
    let cleared = fill_range_filter(
      &subscription,
      &[MessageParameter::new_range_filter_removal(
        RangeFilterKind::Priority,
      )],
    );
    assert!(cleared.matches(fields(0, 200)));
  }

  fn track(value: u64) -> Vec<TrackProperty> {
    vec![TrackProperty::Unknown {
      kvp: moqtail::model::common::pair::KeyValuePair::try_new_varint(0x3C, value).unwrap(),
    }]
  }

  #[test]
  fn subscribe_tracks_takes_track_filters() {
    let params = [filter(
      RangeFilterKind::TrackProperty,
      0,
      &[Range::new(1, 1)],
    )];
    assert!(tracks_filter(&params, 1).is_ok());
    assert!(tracks_filter(&params, 0).is_err());
  }

  #[test]
  fn a_track_is_pushed_with_the_object_filters_of_the_sets_it_passes() {
    let params = [
      // Set 0: property 0x3C of 1, object 0 only.
      filter(RangeFilterKind::TrackProperty, 0, &[Range::new(1, 1)]),
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 0)]),
      // Set 1: property 0x3C of 2, priority 0-63.
      filter(RangeFilterKind::TrackProperty, 1, &[Range::new(2, 2)]),
      filter(RangeFilterKind::Priority, 1, &[Range::new(0, 63)]),
    ];
    assert_eq!(
      filters_for_track(&params, &track(1)).unwrap(),
      vec![params[1].clone()]
    );
    assert_eq!(
      filters_for_track(&params, &track(2)).unwrap(),
      vec![params[3].clone()]
    );
    assert_eq!(filters_for_track(&params, &track(3)), None);
    assert_eq!(filters_for_track(&params, &[]), None);
  }

  #[test]
  fn a_passing_set_with_no_object_filters_lets_everything_through() {
    let params = [
      filter(RangeFilterKind::TrackProperty, 0, &[Range::new(1, 1)]),
      filter(RangeFilterKind::ObjectId, 1, &[Range::new(0, 0)]),
    ];
    assert_eq!(filters_for_track(&params, &track(1)), Some(vec![]));
    // Set 1 has no track filter, so it passes any track, with its object filter.
    assert_eq!(
      filters_for_track(&params, &track(9)).unwrap(),
      vec![params[1].clone()]
    );
  }

  #[test]
  fn no_filters_push_every_track_unfiltered() {
    assert_eq!(filters_for_track(&[], &track(1)), Some(vec![]));
  }

  #[test]
  fn a_removal_is_refused_where_no_filters_are_allowed() {
    let removal = [MessageParameter::new_range_filter_removal(
      RangeFilterKind::ObjectId,
    )];
    assert!(updated_filter(&[], &removal, 0).is_err());
    assert!(updated_filter(&[], &removal, 1).unwrap().is_empty());
  }
}
