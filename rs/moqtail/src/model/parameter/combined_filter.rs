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

use crate::model::common::pair::KeyValuePair;
use crate::model::data::object::Object;
use crate::model::parameter::message_parameter::MessageParameter;
use crate::model::parameter::range_filter::{Range, RangeFilterError, RangeFilterKind};
use crate::model::property::object_property::ObjectProperty;
use crate::model::property::track_property::TrackProperty;
use std::collections::{BTreeMap, HashSet};

/// The criteria one SetID holds. Each one present must pass; an absent one
/// passes anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FilterSet {
  subgroup: Option<Vec<Range>>,
  object_id: Option<Vec<Range>>,
  priority: Option<Vec<Range>>,
  /// Property Type to the Ranges its value must fall in.
  object_properties: BTreeMap<u64, Vec<Range>>,
  track_properties: BTreeMap<u64, Vec<Range>>,
}

/// What an Object is filtered on. An Object whose Subgroup ID is its subgroup's
/// first Object ID cannot always tell it on its own, so the caller supplies it.
#[derive(Debug, Clone, Copy)]
pub struct ObjectFields<'a> {
  /// `None` for a datagram, which has no subgroup and so passes no subgroup filter.
  pub subgroup_id: Option<u64>,
  pub object_id: u64,
  pub publisher_priority: u8,
  pub properties: Option<&'a [ObjectProperty]>,
}

impl<'a> From<&'a Object> for ObjectFields<'a> {
  fn from(object: &'a Object) -> Self {
    Self {
      subgroup_id: object.subgroup_id,
      object_id: object.location.object,
      publisher_priority: object.publisher_priority,
      properties: object.properties.as_deref(),
    }
  }
}

/// What a request's Range Filters add up to.
///
/// Ranges within one filter are OR'd, filters sharing a SetID are AND'd, and the
/// SetIDs are OR'd with each other. No filter at all passes everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CombinedFilter {
  sets: BTreeMap<u8, FilterSet>,
  range_count: u64,
}

impl CombinedFilter {
  /// Builds the filter from every Range Filter in `params`, track filters
  /// included; anything else, and a zero-length removal, is ignored.
  ///
  /// Fails on a filter that cannot be applied: one repeating a Type, SetID and
  /// Property Type already given, an odd Property Type, a priority above 255 or a
  /// bound past 2^64 - 1.
  pub fn new(params: &[MessageParameter]) -> Result<Self, RangeFilterError> {
    let mut filter = Self::default();
    let mut seen = HashSet::new();
    for param in params {
      let MessageParameter::RangeFilter {
        kind,
        filter: Some(range_filter),
      } = param
      else {
        continue;
      };
      let key = (*kind, range_filter.set_id, range_filter.property_type);
      if !seen.insert(key) {
        return Err(RangeFilterError::Duplicate {
          kind: *kind,
          set_id: range_filter.set_id,
          property_type: range_filter.property_type,
        });
      }

      let ranges = range_filter.validate(*kind)?;
      filter.range_count += ranges.len() as u64;
      let set = filter.sets.entry(range_filter.set_id).or_default();
      match kind {
        RangeFilterKind::Subgroup => set.subgroup = Some(ranges),
        RangeFilterKind::ObjectId => set.object_id = Some(ranges),
        RangeFilterKind::Priority => set.priority = Some(ranges),
        RangeFilterKind::ObjectProperty => {
          set
            .object_properties
            .insert(range_filter.property_type.unwrap_or_default(), ranges);
        }
        RangeFilterKind::TrackProperty => {
          set
            .track_properties
            .insert(range_filter.property_type.unwrap_or_default(), ranges);
        }
      }
    }
    Ok(filter)
  }

  /// As [`Self::new`], for a request that chooses Objects, where a track filter
  /// has no place.
  pub fn for_objects(params: &[MessageParameter]) -> Result<Self, RangeFilterError> {
    let filter = Self::new(params)?;
    if filter.has_track_filters() {
      return Err(RangeFilterError::TrackFilterOnObjects);
    }
    Ok(filter)
  }

  /// Whether the filter passes everything.
  pub fn is_empty(&self) -> bool {
    self.sets.is_empty()
  }

  /// How many Ranges the filters hold, which is what a range limit counts.
  pub fn range_count(&self) -> u64 {
    self.range_count
  }

  pub fn has_track_filters(&self) -> bool {
    self
      .sets
      .values()
      .any(|set| !set.track_properties.is_empty())
  }

  /// The object filter a track with `properties` is subscribed with: the sets whose
  /// track criteria it passes, with those criteria dropped. `None` when no set
  /// passes it, so the track is not to be offered at all.
  pub fn narrow_for_track(&self, properties: &[TrackProperty]) -> Option<CombinedFilter> {
    if self.is_empty() {
      return Some(Self::default());
    }
    let values = track_property_values(properties);
    let mut narrowed = Self::default();
    for (set_id, set) in &self.sets {
      if !properties_pass(&set.track_properties, &values) {
        continue;
      }
      let set = FilterSet {
        track_properties: BTreeMap::new(),
        ..set.clone()
      };
      narrowed.range_count += set_range_count(&set);
      narrowed.sets.insert(*set_id, set);
    }
    (!narrowed.sets.is_empty()).then_some(narrowed)
  }

  /// Whether an Object with these fields passes. Track criteria are not looked at:
  /// a filter that has them is narrowed to the track first.
  pub fn matches(&self, fields: ObjectFields<'_>) -> bool {
    if self.is_empty() {
      return true;
    }
    let mut values = None;
    self.sets.values().any(|set| {
      if let Some(ranges) = &set.subgroup
        && !fields.subgroup_id.is_some_and(|id| within(ranges, id))
      {
        return false;
      }
      if let Some(ranges) = &set.object_id
        && !within(ranges, fields.object_id)
      {
        return false;
      }
      if let Some(ranges) = &set.priority
        && !within(ranges, fields.publisher_priority as u64)
      {
        return false;
      }
      if set.object_properties.is_empty() {
        return true;
      }
      let values = values.get_or_insert_with(|| object_property_values(fields.properties));
      properties_pass(&set.object_properties, values)
    })
  }

  pub fn matches_object(&self, object: &Object) -> bool {
    self.matches(ObjectFields::from(object))
  }
}

/// Checks the Range Filters in `params` against a MAX_FILTER_RANGES of `limit`.
///
/// A limit of 0 allows no Range Filter parameter at all, not even a zero-length
/// removal. Otherwise the Ranges of every filter in the request count together.
/// FILL_PARAMETERS describes a separate fill fetch stream, so the filters inside
/// it count against a limit of their own.
pub fn check_filter_range_limit(
  params: &[MessageParameter],
  limit: u64,
) -> Result<(), RangeFilterError> {
  let mut any = false;
  let mut count = 0u64;
  for param in params {
    match param {
      MessageParameter::RangeFilter { filter, .. } => {
        any = true;
        count += filter.as_ref().map_or(0, |f| f.range_count() as u64);
      }
      MessageParameter::FillParameters { parameters } => {
        check_filter_range_limit(parameters, limit)?;
      }
      _ => {}
    }
  }
  if any && (limit == 0 || count > limit) {
    return Err(RangeFilterError::TooManyRanges { count, limit });
  }
  Ok(())
}

fn within(ranges: &[Range], value: u64) -> bool {
  ranges.iter().any(|range| range.contains(value))
}

fn set_range_count(set: &FilterSet) -> u64 {
  [&set.subgroup, &set.object_id, &set.priority]
    .into_iter()
    .flatten()
    .chain(set.object_properties.values())
    .chain(set.track_properties.values())
    .map(|ranges| ranges.len() as u64)
    .sum()
}

/// Whether each required Property is present with a value in its Ranges. One
/// that is missing fails.
fn properties_pass(required: &BTreeMap<u64, Vec<Range>>, values: &BTreeMap<u64, u64>) -> bool {
  required.iter().all(|(property_type, ranges)| {
    values
      .get(property_type)
      .is_some_and(|value| within(ranges, *value))
  })
}

/// The integer-valued Properties an Object carries, those inside
/// ImmutableProperties included. The first of a repeated type counts.
fn object_property_values(properties: Option<&[ObjectProperty]>) -> BTreeMap<u64, u64> {
  let mut values = BTreeMap::new();
  for property in properties.unwrap_or_default() {
    match property {
      ObjectProperty::ImmutableProperties { properties } => collect_kvps(properties, &mut values),
      other => {
        if let Ok(kvp) = TryInto::<KeyValuePair>::try_into(other.clone()) {
          collect_kvps(std::slice::from_ref(&kvp), &mut values);
        }
      }
    }
  }
  values
}

fn track_property_values(properties: &[TrackProperty]) -> BTreeMap<u64, u64> {
  let mut values = BTreeMap::new();
  for property in properties {
    match property {
      TrackProperty::ImmutableProperties { properties } => collect_kvps(properties, &mut values),
      other => {
        if let Ok(kvp) = TryInto::<KeyValuePair>::try_into(other.clone()) {
          collect_kvps(std::slice::from_ref(&kvp), &mut values);
        }
      }
    }
  }
  values
}

fn collect_kvps(kvps: &[KeyValuePair], values: &mut BTreeMap<u64, u64>) {
  for kvp in kvps {
    if let KeyValuePair::VarInt { type_value, value } = kvp {
      values.entry(*type_value).or_insert(*value);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::parameter::range_filter::RangeFilter;

  fn filter(kind: RangeFilterKind, set_id: u8, ranges: &[Range]) -> MessageParameter {
    let property_type = kind.has_property_type().then_some(0x3C);
    MessageParameter::new_range_filter(
      kind,
      RangeFilter::new(set_id, property_type, ranges).unwrap(),
    )
  }

  fn fields(subgroup: Option<u64>, object: u64, priority: u8) -> ObjectFields<'static> {
    ObjectFields {
      subgroup_id: subgroup,
      object_id: object,
      publisher_priority: priority,
      properties: None,
    }
  }

  fn property(value: u64) -> ObjectProperty {
    ObjectProperty::Unknown {
      kvp: KeyValuePair::try_new_varint(0x3C, value).unwrap(),
    }
  }

  #[test]
  fn no_filter_passes_everything() {
    let net = CombinedFilter::new(&[
      MessageParameter::new_forward(true),
      MessageParameter::new_range_filter_removal(RangeFilterKind::ObjectId),
    ])
    .unwrap();
    assert!(net.is_empty());
    assert!(net.matches(fields(None, 9, 0)));
  }

  #[test]
  fn ranges_within_one_filter_are_ored() {
    let net = CombinedFilter::new(&[filter(
      RangeFilterKind::ObjectId,
      0,
      &[Range::new(0, 1), Range::new(5, 5)],
    )])
    .unwrap();
    let passed: Vec<_> = [0, 1, 2, 5, 6]
      .into_iter()
      .map(|id| net.matches(fields(Some(0), id, 0)))
      .collect();
    assert_eq!(passed, [true, true, false, true, false]);
    assert_eq!(net.range_count(), 2);
  }

  #[test]
  fn filters_sharing_a_set_are_anded_and_sets_are_ored() {
    let net = CombinedFilter::new(&[
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 0)]),
      filter(RangeFilterKind::Priority, 0, &[Range::new(0, 63)]),
      filter(RangeFilterKind::Subgroup, 1, &[Range::new(3, 3)]),
    ])
    .unwrap();
    assert!(net.matches(fields(Some(0), 0, 10)));
    assert!(!net.matches(fields(Some(0), 0, 200)));
    assert!(net.matches(fields(Some(3), 4, 200)));
    assert!(!net.matches(fields(Some(0), 4, 10)));
  }

  #[test]
  fn a_subgroup_filter_passes_no_datagram() {
    let net =
      CombinedFilter::new(&[filter(RangeFilterKind::Subgroup, 0, &[Range::open(0)])]).unwrap();
    assert!(net.matches(fields(Some(0), 0, 0)));
    assert!(!net.matches(fields(None, 0, 0)));
  }

  #[test]
  fn a_property_filter_needs_the_property_and_looks_inside_immutable_properties() {
    let net = CombinedFilter::new(&[filter(
      RangeFilterKind::ObjectProperty,
      0,
      &[Range::new(10, 20)],
    )])
    .unwrap();
    let with = |properties: &[ObjectProperty]| {
      net.matches(ObjectFields {
        properties: Some(properties),
        ..fields(Some(0), 0, 0)
      })
    };
    assert!(with(&[property(15)]));
    assert!(!with(&[property(25)]));
    assert!(!with(&[]));
    assert!(!net.matches(fields(Some(0), 0, 0)));
    assert!(with(&[ObjectProperty::ImmutableProperties {
      properties: vec![KeyValuePair::try_new_varint(0x3C, 12).unwrap()],
    }]));
  }

  #[test]
  fn a_repeated_type_set_and_property_is_invalid() {
    let err = CombinedFilter::new(&[
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 0)]),
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(4, 4)]),
    ])
    .unwrap_err();
    assert!(matches!(err, RangeFilterError::Duplicate { set_id: 0, .. }));
    // A different Property Type under the same SetID is a separate filter.
    let other_property = MessageParameter::new_range_filter(
      RangeFilterKind::ObjectProperty,
      RangeFilter::new(0, Some(0x3E), &[Range::open(0)]).unwrap(),
    );
    assert!(
      CombinedFilter::new(&[
        filter(RangeFilterKind::ObjectProperty, 0, &[Range::open(0)]),
        other_property,
      ])
      .is_ok()
    );
  }

  #[test]
  fn a_track_filter_has_no_place_where_objects_are_chosen() {
    let params = [filter(RangeFilterKind::TrackProperty, 0, &[Range::open(0)])];
    assert!(CombinedFilter::new(&params).is_ok());
    assert_eq!(
      CombinedFilter::for_objects(&params),
      Err(RangeFilterError::TrackFilterOnObjects)
    );
  }

  #[test]
  fn a_track_is_offered_with_the_sets_its_properties_pass() {
    let track = |value: u64| {
      vec![TrackProperty::Unknown {
        kvp: KeyValuePair::try_new_varint(0x3C, value).unwrap(),
      }]
    };
    let net = CombinedFilter::new(&[
      // Set 0: tracks with property 0x3C in 1-1, object 0 only.
      filter(RangeFilterKind::TrackProperty, 0, &[Range::new(1, 1)]),
      filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 0)]),
      // Set 1: tracks with property 0x3C in 2-2, everything.
      filter(RangeFilterKind::TrackProperty, 1, &[Range::new(2, 2)]),
    ])
    .unwrap();

    let first = net.narrow_for_track(&track(1)).unwrap();
    assert!(!first.has_track_filters());
    assert!(first.matches(fields(Some(0), 0, 0)));
    assert!(!first.matches(fields(Some(0), 1, 0)));

    // A set left with no criteria passes everything.
    let second = net.narrow_for_track(&track(2)).unwrap();
    assert!(second.matches(fields(Some(0), 7, 0)));

    assert_eq!(net.narrow_for_track(&track(3)), None);
    assert_eq!(net.narrow_for_track(&[]), None);
    assert_eq!(
      CombinedFilter::default().narrow_for_track(&[]),
      Some(CombinedFilter::default())
    );
  }

  #[test]
  fn the_range_limit_counts_a_request_and_its_fill_apart() {
    let two = filter(
      RangeFilterKind::ObjectId,
      0,
      &[Range::new(0, 0), Range::open(4)],
    );
    let one = filter(RangeFilterKind::Subgroup, 0, &[Range::new(1, 1)]);
    let removal = MessageParameter::new_range_filter_removal(RangeFilterKind::Priority);

    assert!(check_filter_range_limit(&[MessageParameter::new_forward(true)], 0).is_ok());
    assert!(check_filter_range_limit(&[two.clone(), one.clone()], 3).is_ok());
    assert!(check_filter_range_limit(&[two.clone(), one.clone()], 2).is_err());
    // A limit of 0 allows not even a removal.
    assert!(check_filter_range_limit(std::slice::from_ref(&removal), 0).is_err());
    assert!(check_filter_range_limit(std::slice::from_ref(&removal), 1).is_ok());

    let fill = MessageParameter::new_fill_parameters(vec![two.clone()]);
    assert!(check_filter_range_limit(&[two.clone(), fill], 2).is_ok());
    let crowded_fill = MessageParameter::new_fill_parameters(vec![two, one]);
    assert!(check_filter_range_limit(&[crowded_fill], 2).is_err());
  }
}
