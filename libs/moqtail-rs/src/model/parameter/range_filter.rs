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

//! Range Filters, parameter types 0x25-0x29 (§5.1.4).
//!
//! Five parameters share one body, so they share one type here and carry which
//! they are in [`RangeFilterKind`]. Two of them (the property filters) add a
//! Property Type between the SetID and the Ranges; nothing else differs on the
//! wire.
//!
//! Almost every "MUST reject" in §5.1.4 is a REQUEST_ERROR with INVALID_FILTER,
//! not a session-closing offence, so it is **not** in the decoder:
//! [`validate_range_filters`] is the one place that decides, and it is called
//! where a request is answered. The decoder refuses only what does not match the
//! Type's serialization at all (§1.4.3), which is a formatting error.

use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use crate::model::parameter::constant::MessageParameterType;
use crate::model::parameter::message_parameter::MessageParameter;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::collections::HashSet;

/// Which of the five Range Filter parameters this is. The filtered quantity is
/// the only difference in meaning; the body is identical apart from the Property
/// Type the two property filters carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeFilterKind {
  /// §10.2.10, Subgroup ID.
  Subgroup,
  /// §10.2.11, Object ID.
  ObjectId,
  /// §10.2.12, Publisher Priority. An 8-bit field, so a value past 255 is
  /// INVALID_FILTER.
  Priority,
  /// §10.2.13, an Object Property named by Property Type.
  ObjectProperty,
  /// §10.2.14, a Track Property named by Property Type. Filters whole PUBLISH
  /// messages rather than Objects.
  TrackProperty,
}

impl RangeFilterKind {
  pub const fn type_value(self) -> u64 {
    match self {
      Self::Subgroup => MessageParameterType::SubgroupFilter as u64,
      Self::ObjectId => MessageParameterType::ObjectIdFilter as u64,
      Self::Priority => MessageParameterType::PriorityFilter as u64,
      Self::ObjectProperty => MessageParameterType::ObjectPropertyFilter as u64,
      Self::TrackProperty => MessageParameterType::TrackPropertyFilter as u64,
    }
  }

  pub const fn from_type_value(value: u64) -> Option<Self> {
    match value {
      v if v == MessageParameterType::SubgroupFilter as u64 => Some(Self::Subgroup),
      v if v == MessageParameterType::ObjectIdFilter as u64 => Some(Self::ObjectId),
      v if v == MessageParameterType::PriorityFilter as u64 => Some(Self::Priority),
      v if v == MessageParameterType::ObjectPropertyFilter as u64 => Some(Self::ObjectProperty),
      v if v == MessageParameterType::TrackPropertyFilter as u64 => Some(Self::TrackProperty),
      _ => None,
    }
  }

  /// The two property filters carry a Property Type (vi64) after the SetID.
  pub const fn has_property_type(self) -> bool {
    matches!(self, Self::ObjectProperty | Self::TrackProperty)
  }
}

/// One inclusive Start/End pair. `end` is absent only on a filter's last Range,
/// where it means open-ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
  pub start: u64,
  pub end: Option<u64>,
}

impl Range {
  pub const fn new(start: u64, end: u64) -> Self {
    Self {
      start,
      end: Some(end),
    }
  }

  /// The open-ended form, legal only as a filter's last Range.
  pub const fn open(start: u64) -> Self {
    Self { start, end: None }
  }

  pub const fn contains(&self, value: u64) -> bool {
    value >= self.start
      && match self.end {
        Some(end) => value <= end,
        None => true,
      }
  }
}

/// A Range Filter parameter (§5.1.4).
///
/// ```text
/// SUBGROUP_FILTER { Type (vi64) = 0x25, Length (vi64), [SetID (8)], [Range (..) ...] }
/// OBJECT_PROPERTY_FILTER { Type = 0x28, Length, [SetID (8)], [Property Type (vi64)], [Range ...] }
/// Range { Start (vi64), [End (vi64)] }
/// ```
///
/// Ranges are stored absolute and inclusive; the wire delta-codes them, Start
/// from the previous Range's End and End from its own Start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeFilter {
  pub kind: RangeFilterKind,
  /// `None` is the zero-length form: Length 0, no further fields, no filter at
  /// all. That is a real value and not an absent parameter -- it is how
  /// REQUEST_UPDATE removes a filter.
  ///
  /// Otherwise: filters sharing a SetID are AND'd and distinct SetIDs are OR'd,
  /// so the result is `SetID=0 OR SetID=1 OR ... SetID=255`.
  pub set_id: Option<u8>,
  /// Present exactly when `kind.has_property_type()` and the filter is not the
  /// zero-length form.
  pub property_type: Option<u64>,
  /// Ranges are OR'd within one filter.
  pub ranges: Vec<Range>,
  /// A Start or End delta summed past 2^64 - 1. §5.1.4 makes that a
  /// REQUEST_ERROR with INVALID_FILTER rather than a session close, and an
  /// overflowing sum has no `u64` to be stored in, so the decoder records it
  /// here, keeps what it decoded first, and leaves the verdict to
  /// [`validate_range_filters`].
  pub delta_overflow: bool,
}

impl RangeFilter {
  /// The zero-length form: removes a filter in REQUEST_UPDATE, and filters
  /// nothing anywhere else.
  pub const fn empty(kind: RangeFilterKind) -> Self {
    Self {
      kind,
      set_id: None,
      property_type: None,
      ranges: Vec::new(),
      delta_overflow: false,
    }
  }

  /// A filter on one of the three header-field kinds. Refuses a property kind,
  /// which needs [`Self::new_property`].
  pub fn new(kind: RangeFilterKind, set_id: u8, ranges: Vec<Range>) -> Result<Self, ParseError> {
    if kind.has_property_type() {
      return Err(ParseError::ProtocolViolation {
        context: "RangeFilter::new",
        details: format!("{kind:?} requires a Property Type"),
      });
    }
    Self::build(kind, set_id, None, ranges)
  }

  /// A filter on an Object or Track Property.
  pub fn new_property(
    kind: RangeFilterKind,
    set_id: u8,
    property_type: u64,
    ranges: Vec<Range>,
  ) -> Result<Self, ParseError> {
    if !kind.has_property_type() {
      return Err(ParseError::ProtocolViolation {
        context: "RangeFilter::new_property",
        details: format!("{kind:?} carries no Property Type"),
      });
    }
    Self::build(kind, set_id, Some(property_type), ranges)
  }

  fn build(
    kind: RangeFilterKind,
    set_id: u8,
    property_type: Option<u64>,
    ranges: Vec<Range>,
  ) -> Result<Self, ParseError> {
    // Only the last Range may be open-ended: an unbounded Range in the middle
    // leaves the next Start nothing to be a delta from.
    if let Some(index) = ranges.iter().position(|r| r.end.is_none())
      && index + 1 != ranges.len()
    {
      return Err(ParseError::ProtocolViolation {
        context: "RangeFilter::new",
        details: format!(
          "open-ended Range at index {index} is not the last of {}",
          ranges.len()
        ),
      });
    }
    // Start is delta-coded from the previous End and End from its own Start, and
    // a delta is unsigned, so Ranges must ascend and not overlap. Caught here, at
    // the mistake, rather than only where it would wrap on the wire.
    let mut prev_end = 0u64;
    for (index, range) in ranges.iter().enumerate() {
      if range.start < prev_end {
        return Err(ParseError::ProtocolViolation {
          context: "RangeFilter::new",
          details: format!(
            "Range {index} starts at {} before the previous End {prev_end}",
            range.start
          ),
        });
      }
      if let Some(end) = range.end {
        if end < range.start {
          return Err(ParseError::ProtocolViolation {
            context: "RangeFilter::new",
            details: format!(
              "Range {index} ends at {end} before its own Start {}",
              range.start
            ),
          });
        }
        prev_end = end;
      }
    }
    Ok(Self {
      kind,
      set_id: Some(set_id),
      property_type,
      ranges,
      delta_overflow: false,
    })
  }

  /// The zero-length form.
  pub const fn is_empty(&self) -> bool {
    self.set_id.is_none()
  }

  /// Whether `value` falls in any Range -- Ranges are OR'd within one filter.
  ///
  /// An empty filter is no filter, so everything passes. A filter that states a
  /// SetID and no Ranges passes nothing, which is what "falls within any Range"
  /// says when there are none.
  pub fn matches(&self, value: u64) -> bool {
    if self.is_empty() {
      return true;
    }
    self.ranges.iter().any(|r| r.contains(value))
  }

  /// The identity §5.1.4 forbids repeating within one message: Parameter Type,
  /// SetID, and Property Type.
  pub fn dedup_key(&self) -> (u64, Option<u8>, Option<u64>) {
    (self.kind.type_value(), self.set_id, self.property_type)
  }

  pub fn serialize_value(&self) -> Result<Bytes, ParseError> {
    if self.delta_overflow {
      return Err(ParseError::ProtocolViolation {
        context: "RangeFilter::serialize_value",
        details: "refusing to re-emit a filter whose deltas overflowed on receipt".to_string(),
      });
    }
    let Some(set_id) = self.set_id else {
      return Ok(Bytes::new());
    };
    let mut buf = BytesMut::new();
    buf.put_u8(set_id);
    if let Some(property_type) = self.property_type {
      buf.put_vi(property_type)?;
    }
    let mut prev_end = 0u64;
    for (index, range) in self.ranges.iter().enumerate() {
      let start_delta =
        range
          .start
          .checked_sub(prev_end)
          .ok_or_else(|| ParseError::ProtocolViolation {
            context: "RangeFilter::serialize_value",
            details: format!(
              "Range {index} starts at {} before the previous End {prev_end}; Start is delta-coded from it",
              range.start
            ),
          })?;
      buf.put_vi(start_delta)?;
      match range.end {
        Some(end) => {
          let end_delta =
            end
              .checked_sub(range.start)
              .ok_or_else(|| ParseError::ProtocolViolation {
                context: "RangeFilter::serialize_value",
                details: format!(
                  "Range {index} ends at {end} before its own Start {}",
                  range.start
                ),
              })?;
          buf.put_vi(end_delta)?;
          prev_end = end;
        }
        None if index + 1 == self.ranges.len() => {}
        None => {
          return Err(ParseError::ProtocolViolation {
            context: "RangeFilter::serialize_value",
            details: format!("open-ended Range at index {index} is not the last"),
          });
        }
      }
    }
    Ok(buf.freeze())
  }

  pub fn parse_value(kind: RangeFilterKind, value: &Bytes) -> Result<Self, ParseError> {
    let mut payload = value.clone();
    if !payload.has_remaining() {
      return Ok(Self::empty(kind));
    }
    let set_id = payload.get_u8();
    let property_type = if kind.has_property_type() {
      Some(payload.get_vi()?)
    } else {
      None
    };

    let mut ranges = Vec::new();
    let mut delta_overflow = false;
    let mut prev_end = 0u64;
    while payload.has_remaining() {
      let Some(start) = prev_end.checked_add(payload.get_vi()?) else {
        delta_overflow = true;
        break;
      };
      if !payload.has_remaining() {
        // No End left to read, so this last Range is open-ended.
        ranges.push(Range::open(start));
        break;
      }
      let Some(end) = start.checked_add(payload.get_vi()?) else {
        delta_overflow = true;
        break;
      };
      ranges.push(Range::new(start, end));
      prev_end = end;
    }

    Ok(Self {
      kind,
      set_id: Some(set_id),
      property_type,
      ranges,
      delta_overflow,
    })
  }
}

/// Why a request's Range Filters must be answered with REQUEST_ERROR carrying
/// INVALID_FILTER.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidFilter(pub String);

/// The §5.1.4 rules that need either more than one parameter or the session's
/// MAX_FILTER_RANGES to decide, plus the two per-filter rules the decoder cannot
/// enforce without turning a request error into a session close.
///
/// `max_filter_ranges` is the value **this** endpoint advertised in its Setup:
/// the option limits what the peer may send (§10.3.1.6). Its default is 0, and 0
/// does not mean unlimited -- it prohibits Range Filters outright, which is why
/// an endpoint that never sends the option must reject every one it receives.
pub fn validate_range_filters(
  params: &[MessageParameter],
  max_filter_ranges: u64,
) -> Result<(), InvalidFilter> {
  let filters: Vec<&RangeFilter> = params
    .iter()
    .filter_map(|p| match p {
      MessageParameter::RangeFilter(filter) => Some(filter),
      _ => None,
    })
    .collect();
  if filters.is_empty() {
    return Ok(());
  }
  if max_filter_ranges == 0 {
    return Err(InvalidFilter(
      "MAX_FILTER_RANGES is 0, so no Range Filter parameter may be sent".to_string(),
    ));
  }

  let mut total_ranges = 0u64;
  let mut seen = HashSet::new();
  for filter in filters {
    if filter.delta_overflow {
      return Err(InvalidFilter(format!(
        "a Range delta in parameter 0x{:02X} sums past 2^64 - 1",
        filter.kind.type_value()
      )));
    }
    if !seen.insert(filter.dedup_key()) {
      return Err(InvalidFilter(format!(
        "parameter 0x{:02X} repeats SetID {:?} and Property Type {:?}",
        filter.kind.type_value(),
        filter.set_id,
        filter.property_type
      )));
    }
    if let Some(property_type) = filter.property_type
      && !property_type.is_multiple_of(2)
    {
      return Err(InvalidFilter(format!(
        "Property Type {property_type} is odd, so it is not a single integer value"
      )));
    }
    if filter.kind == RangeFilterKind::Priority
      && let Some(value) = filter
        .ranges
        .iter()
        .flat_map(|r| [Some(r.start), r.end])
        .flatten()
        .find(|v| *v > 255)
    {
      return Err(InvalidFilter(format!(
        "PRIORITY_FILTER value {value} exceeds 255; Publisher Priority is an 8-bit field"
      )));
    }
    total_ranges = total_ranges.saturating_add(filter.ranges.len() as u64);
  }
  if total_ranges > max_filter_ranges {
    return Err(InvalidFilter(format!(
      "{total_ranges} Ranges exceed the advertised MAX_FILTER_RANGES of {max_filter_ranges}"
    )));
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn filter(ranges: Vec<Range>) -> RangeFilter {
    RangeFilter::new(RangeFilterKind::Subgroup, 0, ranges).unwrap()
  }

  // The draft's own worked example: "ranges 3-5 and 10-15 encode as: Start=3,
  // End=2, Start=5, End=5".
  #[test]
  fn the_draft_worked_example_encodes_byte_for_byte() {
    let f = filter(vec![Range::new(3, 5), Range::new(10, 15)]);
    assert_eq!(
      f.serialize_value().unwrap().to_vec(),
      vec![0u8, 3, 2, 5, 5],
      "SetID then Start=3, End=2, Start=5, End=5"
    );
    assert_eq!(
      RangeFilter::parse_value(RangeFilterKind::Subgroup, &f.serialize_value().unwrap()).unwrap(),
      f
    );
  }

  #[test]
  fn a_zero_length_value_is_the_no_filter_form_and_not_an_absent_parameter() {
    let empty = RangeFilter::empty(RangeFilterKind::ObjectId);
    assert!(empty.serialize_value().unwrap().is_empty());
    let parsed = RangeFilter::parse_value(RangeFilterKind::ObjectId, &Bytes::new()).unwrap();
    assert!(parsed.is_empty());
    assert_eq!(parsed, empty);
    // No filter passes everything; a stated filter with no Ranges passes nothing.
    assert!(empty.matches(7));
    assert!(!filter(vec![]).matches(7));
  }

  #[test]
  fn the_last_range_may_be_open_ended_and_an_earlier_one_may_not() {
    let f = filter(vec![Range::new(1, 2), Range::open(9)]);
    assert_eq!(f.serialize_value().unwrap().to_vec(), vec![0u8, 1, 1, 7]);
    let parsed =
      RangeFilter::parse_value(RangeFilterKind::Subgroup, &f.serialize_value().unwrap()).unwrap();
    assert_eq!(parsed, f);
    assert!(parsed.matches(u64::MAX));
    assert!(
      RangeFilter::new(
        RangeFilterKind::Subgroup,
        0,
        vec![Range::open(1), Range::new(5, 6)]
      )
      .is_err()
    );
  }

  // The fields are public, so a caller can still hand serialization a descending
  // or overlapping set. Both deltas are unsigned, so that must be an error rather
  // than a wrap -- a wrapped Start encodes a range nobody asked for.
  #[test]
  fn ranges_must_ascend_and_not_overlap() {
    assert!(
      RangeFilter::new(
        RangeFilterKind::Subgroup,
        0,
        vec![Range::new(10, 15), Range::new(3, 5)]
      )
      .is_err()
    );
    assert!(
      RangeFilter::new(
        RangeFilterKind::Subgroup,
        0,
        vec![Range::new(3, 9), Range::new(5, 11)]
      )
      .is_err()
    );
    assert!(RangeFilter::new(RangeFilterKind::Subgroup, 0, vec![Range::new(5, 3)]).is_err());

    let mut descending = filter(vec![Range::new(3, 5)]);
    descending.ranges = vec![Range::new(10, 15), Range::new(3, 5)];
    assert!(descending.serialize_value().is_err());
    let mut backwards = filter(vec![Range::new(3, 5)]);
    backwards.ranges = vec![Range::new(5, 3)];
    assert!(backwards.serialize_value().is_err());
  }

  #[test]
  fn ranges_are_or_d_within_a_filter_and_inclusive_at_both_ends() {
    let f = filter(vec![Range::new(3, 5), Range::new(10, 15)]);
    assert!(!f.matches(2));
    assert!(f.matches(3));
    assert!(f.matches(5));
    assert!(!f.matches(6));
    assert!(f.matches(10));
    assert!(f.matches(15));
    assert!(!f.matches(16));
  }

  #[test]
  fn a_property_filter_carries_its_property_type_between_set_id_and_ranges() {
    let f = RangeFilter::new_property(
      RangeFilterKind::ObjectProperty,
      7,
      4,
      vec![Range::new(1, 1)],
    )
    .unwrap();
    assert_eq!(f.serialize_value().unwrap().to_vec(), vec![7u8, 4, 1, 0]);
    assert_eq!(
      RangeFilter::parse_value(
        RangeFilterKind::ObjectProperty,
        &f.serialize_value().unwrap()
      )
      .unwrap(),
      f
    );
    // The two kinds disagree about the field, so each refuses the other's shape.
    assert!(RangeFilter::new(RangeFilterKind::ObjectProperty, 0, vec![]).is_err());
    assert!(RangeFilter::new_property(RangeFilterKind::Subgroup, 0, 2, vec![]).is_err());
  }

  // The Property Type is read only for 0x28/0x29, so the same bytes mean different
  // things under the two shapes: here the 4 that was a Property Type becomes the
  // first Start, and everything after it shifts by one field. Reading one as the
  // other is the desync this pins.
  #[test]
  fn the_property_type_is_not_read_for_a_header_field_filter() {
    let bytes = Bytes::from_static(&[7u8, 4, 1, 0]);
    let as_subgroup = RangeFilter::parse_value(RangeFilterKind::Subgroup, &bytes).unwrap();
    assert_eq!(as_subgroup.property_type, None);
    assert_eq!(as_subgroup.ranges, vec![Range::new(4, 5), Range::open(5)]);
  }

  // Both sums can overflow and they are separate adds: End is Start plus its own
  // delta, Start is the previous End plus its own. A test that only reaches one
  // leaves the other free to wrap silently.
  #[test]
  fn either_delta_summing_past_2_64_is_recorded_rather_than_closing_the_session() {
    // End overflows: Start is already u64::MAX, so any non-zero End delta is past it.
    let mut end_overflow = BytesMut::new();
    end_overflow.put_u8(0);
    end_overflow.put_vi(u64::MAX).unwrap();
    end_overflow.put_vi(1u64).unwrap();

    // Start overflows: the first Range ends at u64::MAX, so the next Start cannot
    // be a non-zero delta from it.
    let mut start_overflow = BytesMut::new();
    start_overflow.put_u8(0);
    start_overflow.put_vi(u64::MAX).unwrap();
    start_overflow.put_vi(0u64).unwrap();
    start_overflow.put_vi(1u64).unwrap();

    for value in [end_overflow.freeze(), start_overflow.freeze()] {
      let parsed = RangeFilter::parse_value(RangeFilterKind::Subgroup, &value).unwrap();
      assert!(
        parsed.delta_overflow,
        "parse must succeed so the request can be answered"
      );
      assert!(
        parsed.serialize_value().is_err(),
        "but we never re-emit one"
      );
      assert_eq!(
        validate_range_filters(&[MessageParameter::RangeFilter(parsed)], 16),
        Err(InvalidFilter(
          "a Range delta in parameter 0x25 sums past 2^64 - 1".to_string()
        ))
      );
    }
  }

  #[test]
  fn max_filter_ranges_of_zero_prohibits_range_filters_outright() {
    let params = vec![MessageParameter::RangeFilter(filter(vec![Range::new(
      1, 2,
    )]))];
    assert!(validate_range_filters(&params, 0).is_err());
    assert!(validate_range_filters(&params, 1).is_ok());
    assert!(validate_range_filters(&[], 0).is_ok());
    // Even the zero-length removal form is "such a filter parameter".
    assert!(
      validate_range_filters(
        &[MessageParameter::RangeFilter(RangeFilter::empty(
          RangeFilterKind::Subgroup
        ))],
        0
      )
      .is_err()
    );
  }

  #[test]
  fn the_limit_counts_ranges_across_every_filter_in_the_request() {
    let params = vec![
      MessageParameter::RangeFilter(filter(vec![Range::new(1, 2), Range::new(4, 5)])),
      MessageParameter::RangeFilter(
        RangeFilter::new(RangeFilterKind::ObjectId, 0, vec![Range::new(1, 2)]).unwrap(),
      ),
    ];
    assert!(validate_range_filters(&params, 3).is_ok());
    assert!(validate_range_filters(&params, 2).is_err());
  }

  #[test]
  fn repeating_type_set_id_and_property_type_is_refused() {
    let one = RangeFilter::new_property(RangeFilterKind::TrackProperty, 1, 2, vec![]).unwrap();
    let same_key = RangeFilter::new_property(RangeFilterKind::TrackProperty, 1, 2, vec![]).unwrap();
    let other_set =
      RangeFilter::new_property(RangeFilterKind::TrackProperty, 2, 2, vec![]).unwrap();
    let other_property =
      RangeFilter::new_property(RangeFilterKind::TrackProperty, 1, 4, vec![]).unwrap();
    let dup = vec![
      MessageParameter::RangeFilter(one.clone()),
      MessageParameter::RangeFilter(same_key),
    ];
    assert!(validate_range_filters(&dup, 16).is_err());
    // Repetition is allowed; it is this exact combination that is not.
    let distinct = vec![
      MessageParameter::RangeFilter(one),
      MessageParameter::RangeFilter(other_set),
      MessageParameter::RangeFilter(other_property),
    ];
    assert!(validate_range_filters(&distinct, 16).is_ok());
  }

  #[test]
  fn an_odd_property_type_is_refused_because_it_is_not_a_single_integer() {
    let odd = RangeFilter::new_property(RangeFilterKind::ObjectProperty, 0, 3, vec![]).unwrap();
    assert!(validate_range_filters(&[MessageParameter::RangeFilter(odd)], 16).is_err());
    let even = RangeFilter::new_property(RangeFilterKind::ObjectProperty, 0, 4, vec![]).unwrap();
    assert!(validate_range_filters(&[MessageParameter::RangeFilter(even)], 16).is_ok());
  }

  #[test]
  fn a_priority_past_255_is_refused_and_only_for_the_priority_filter() {
    let over = RangeFilter::new(RangeFilterKind::Priority, 0, vec![Range::new(250, 300)]).unwrap();
    assert!(validate_range_filters(&[MessageParameter::RangeFilter(over)], 16).is_err());
    let open = RangeFilter::new(RangeFilterKind::Priority, 0, vec![Range::open(256)]).unwrap();
    assert!(validate_range_filters(&[MessageParameter::RangeFilter(open)], 16).is_err());
    let within = RangeFilter::new(RangeFilterKind::Priority, 0, vec![Range::new(0, 255)]).unwrap();
    assert!(validate_range_filters(&[MessageParameter::RangeFilter(within)], 16).is_ok());
    // The same values are ordinary Subgroup IDs.
    assert!(
      validate_range_filters(
        &[MessageParameter::RangeFilter(filter(vec![Range::new(
          250, 300
        )]))],
        16
      )
      .is_ok()
    );
  }
}
