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

use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use crate::model::parameter::constant::MessageParameterType;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::fmt;

/// What a Range Filter selects on. Each kind is its own parameter type, and the
/// two property kinds also name the Property they read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RangeFilterKind {
  Subgroup,
  ObjectId,
  /// Publisher Priority, which is a single byte.
  Priority,
  ObjectProperty,
  /// Selects tracks rather than Objects, so it only rides SUBSCRIBE_TRACKS.
  TrackProperty,
}

impl RangeFilterKind {
  pub const ALL: [RangeFilterKind; 5] = [
    Self::Subgroup,
    Self::ObjectId,
    Self::Priority,
    Self::ObjectProperty,
    Self::TrackProperty,
  ];

  pub fn parameter_type(self) -> MessageParameterType {
    match self {
      Self::Subgroup => MessageParameterType::SubgroupFilter,
      Self::ObjectId => MessageParameterType::ObjectIdFilter,
      Self::Priority => MessageParameterType::PriorityFilter,
      Self::ObjectProperty => MessageParameterType::ObjectPropertyFilter,
      Self::TrackProperty => MessageParameterType::TrackPropertyFilter,
    }
  }

  pub fn from_parameter_type(param_type: MessageParameterType) -> Option<Self> {
    Self::ALL
      .into_iter()
      .find(|kind| kind.parameter_type() == param_type)
  }

  /// Whether the filter carries a Property Type after its SetID.
  pub fn has_property_type(self) -> bool {
    matches!(self, Self::ObjectProperty | Self::TrackProperty)
  }
}

/// An inclusive range of values. An open-ended range has no `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
  pub start: u64,
  pub end: Option<u64>,
}

impl Range {
  pub fn new(start: u64, end: u64) -> Self {
    Self {
      start,
      end: Some(end),
    }
  }

  pub fn open(start: u64) -> Self {
    Self { start, end: None }
  }

  pub fn contains(&self, value: u64) -> bool {
    value >= self.start && self.end.is_none_or(|end| value <= end)
  }
}

/// Why a Range Filter cannot be honoured. A peer that sends one is answered with
/// REQUEST_ERROR INVALID_FILTER, not a closed session: the request is
/// well-formed, it just asks for something that cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeFilterError {
  /// Adding a delta back went past 2^64 - 1.
  Overflow,
  /// Publisher Priority is one byte, so nothing above 255 can match.
  PriorityOutOfRange { value: u64 },
  /// Only even Property Types hold a single integer to compare against.
  OddPropertyType { property_type: u64 },
  /// The filter kind and the presence of a Property Type disagree.
  PropertyTypeMismatch { kind: RangeFilterKind },
  /// Ranges must ascend, and only the last may be open-ended, or the deltas
  /// that carry them cannot be written.
  Unordered,
  /// The same Type, SetID and Property Type appear twice in one request.
  Duplicate {
    kind: RangeFilterKind,
    set_id: u8,
    property_type: Option<u64>,
  },
  /// A track filter where Objects are being chosen.
  TrackFilterOnObjects,
  /// More Ranges than the limit the receiver advertised.
  TooManyRanges { count: u64, limit: u64 },
}

impl fmt::Display for RangeFilterError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Overflow => write!(f, "a range bound exceeds 2^64 - 1"),
      Self::PriorityOutOfRange { value } => {
        write!(f, "priority {value} is outside 0-255")
      }
      Self::OddPropertyType { property_type } => {
        write!(f, "property type {property_type:#x} is odd")
      }
      Self::PropertyTypeMismatch { kind } => {
        write!(f, "{kind:?} filter has the wrong Property Type presence")
      }
      Self::Unordered => write!(f, "ranges are not ascending"),
      Self::Duplicate {
        kind,
        set_id,
        property_type,
      } => match property_type {
        Some(property_type) => write!(
          f,
          "{kind:?} filter repeats SetID {set_id} for property {property_type:#x}"
        ),
        None => write!(f, "{kind:?} filter repeats SetID {set_id}"),
      },
      Self::TrackFilterOnObjects => {
        write!(f, "a track property filter selects tracks, not Objects")
      }
      Self::TooManyRanges { count, limit } => {
        if *limit == 0 {
          write!(f, "no range filters are allowed (MAX_FILTER_RANGES is 0)")
        } else {
          write!(
            f,
            "{count} filter ranges exceed MAX_FILTER_RANGES of {limit}"
          )
        }
      }
    }
  }
}

impl std::error::Error for RangeFilterError {}

/// One Range Filter parameter's value: the set it belongs to, the Property it
/// reads (property kinds only), and its Ranges.
///
/// The Ranges are held as the deltas the wire carries — each Start from the End
/// before it, each End from its own Start — and only resolved by
/// [`Self::ranges`]. A sum past 2^64 - 1 has to be answered with a REQUEST_ERROR
/// for the request that carried it, and a parser that rejected it would lose the
/// request it belonged to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeFilter {
  pub set_id: u8,
  pub property_type: Option<u64>,
  deltas: Vec<(u64, Option<u64>)>,
}

impl RangeFilter {
  /// Encodes ascending `ranges`. Only the last may be open-ended.
  pub fn new(
    set_id: u8,
    property_type: Option<u64>,
    ranges: &[Range],
  ) -> Result<Self, RangeFilterError> {
    let mut deltas = Vec::with_capacity(ranges.len());
    let mut prior_end = 0u64;
    for (i, range) in ranges.iter().enumerate() {
      let start = range
        .start
        .checked_sub(prior_end)
        .ok_or(RangeFilterError::Unordered)?;
      let end = match range.end {
        Some(end) => {
          prior_end = end;
          Some(
            end
              .checked_sub(range.start)
              .ok_or(RangeFilterError::Unordered)?,
          )
        }
        None if i + 1 == ranges.len() => None,
        None => return Err(RangeFilterError::Unordered),
      };
      deltas.push((start, end));
    }
    Ok(Self {
      set_id,
      property_type,
      deltas,
    })
  }

  /// Takes the deltas as they go on the wire, unchecked. For sending a filter a
  /// peer has to reject.
  pub fn from_deltas(
    set_id: u8,
    property_type: Option<u64>,
    deltas: Vec<(u64, Option<u64>)>,
  ) -> Self {
    Self {
      set_id,
      property_type,
      deltas,
    }
  }

  pub fn deltas(&self) -> &[(u64, Option<u64>)] {
    &self.deltas
  }

  /// How many Ranges the filter holds, which is what a peer's range limit counts.
  pub fn range_count(&self) -> usize {
    self.deltas.len()
  }

  /// The Ranges with their deltas added back.
  pub fn ranges(&self) -> Result<Vec<Range>, RangeFilterError> {
    let mut ranges = Vec::with_capacity(self.deltas.len());
    let mut prior_end = 0u64;
    for (start_delta, end_delta) in &self.deltas {
      let start = prior_end
        .checked_add(*start_delta)
        .ok_or(RangeFilterError::Overflow)?;
      let end = match end_delta {
        Some(delta) => {
          let end = start
            .checked_add(*delta)
            .ok_or(RangeFilterError::Overflow)?;
          prior_end = end;
          Some(end)
        }
        None => None,
      };
      ranges.push(Range { start, end });
    }
    Ok(ranges)
  }

  /// The Ranges, once everything a `kind` filter has to satisfy holds.
  pub fn validate(&self, kind: RangeFilterKind) -> Result<Vec<Range>, RangeFilterError> {
    match (kind.has_property_type(), self.property_type) {
      (true, Some(property_type)) if property_type % 2 != 0 => {
        return Err(RangeFilterError::OddPropertyType { property_type });
      }
      (true, Some(_)) | (false, None) => {}
      _ => return Err(RangeFilterError::PropertyTypeMismatch { kind }),
    }
    let ranges = self.ranges()?;
    if kind == RangeFilterKind::Priority
      && let Some(value) = ranges
        .iter()
        .flat_map(|r| [Some(r.start), r.end])
        .flatten()
        .find(|v| *v > u8::MAX as u64)
    {
      return Err(RangeFilterError::PriorityOutOfRange { value });
    }
    Ok(ranges)
  }

  /// Encodes the SetID, the Property Type if any, and the Ranges. The Parameter
  /// Type and Length are not included.
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_u8(self.set_id);
    if let Some(property_type) = self.property_type {
      buf.put_vi(property_type)?;
    }
    for (i, (start, end)) in self.deltas.iter().enumerate() {
      buf.put_vi(*start)?;
      match end {
        Some(end) => buf.put_vi(*end)?,
        // An End can only be left off the last Range: the next Start would be
        // read as it.
        None if i + 1 == self.deltas.len() => {}
        None => {
          return Err(ParseError::ProtocolViolation {
            context: "RangeFilter::serialize",
            details: "only the last range can be open-ended".to_string(),
          });
        }
      }
    }
    Ok(buf.freeze())
  }

  /// Decodes a non-empty Range Filter value. The Value ends with the last
  /// Range, so a Start with nothing after it is an open-ended last Range.
  pub fn deserialize(kind: RangeFilterKind, bytes: &mut Bytes) -> Result<Self, ParseError> {
    if !bytes.has_remaining() {
      return Err(ParseError::NotEnoughBytes {
        context: "RangeFilter::deserialize(SetID)",
        needed: 1,
        available: 0,
      });
    }
    let set_id = bytes.get_u8();
    let property_type = if kind.has_property_type() {
      Some(bytes.get_vi()?)
    } else {
      None
    };
    let mut deltas = Vec::new();
    while bytes.has_remaining() {
      let start = bytes.get_vi()?;
      let end = if bytes.has_remaining() {
        Some(bytes.get_vi()?)
      } else {
        None
      };
      deltas.push((start, end));
    }
    Ok(Self {
      set_id,
      property_type,
      deltas,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn roundtrip(kind: RangeFilterKind, filter: RangeFilter) {
    let mut bytes = filter.serialize().unwrap();
    let decoded = RangeFilter::deserialize(kind, &mut bytes).unwrap();
    assert_eq!(decoded, filter);
    assert!(!bytes.has_remaining());
  }

  #[test]
  fn ranges_are_delta_encoded_from_the_prior_end() {
    // 3-5 and 10-15 go on the wire as Start=3, End=2, Start=5, End=5.
    let filter = RangeFilter::new(0, None, &[Range::new(3, 5), Range::new(10, 15)]).unwrap();
    assert_eq!(filter.deltas(), &[(3, Some(2)), (5, Some(5))]);
    assert_eq!(filter.serialize().unwrap().as_ref(), &[0x00, 3, 2, 5, 5]);
    assert_eq!(
      filter.ranges().unwrap(),
      vec![Range::new(3, 5), Range::new(10, 15)]
    );
  }

  #[test]
  fn a_start_with_nothing_after_it_is_open_ended() {
    let mut wire = Bytes::from_static(&[0x07, 3, 2, 5]);
    let filter = RangeFilter::deserialize(RangeFilterKind::Subgroup, &mut wire).unwrap();
    assert_eq!(filter.set_id, 7);
    assert_eq!(
      filter.ranges().unwrap(),
      vec![Range::new(3, 5), Range::open(10)]
    );
  }

  #[test]
  fn a_property_filter_names_its_property_after_the_set_id() {
    let filter = RangeFilter::new(2, Some(0x3C), &[Range::new(10, 20)]).unwrap();
    assert_eq!(filter.serialize().unwrap().as_ref(), &[0x02, 0x3C, 10, 10]);
    roundtrip(RangeFilterKind::ObjectProperty, filter);
  }

  #[test]
  fn every_kind_roundtrips() {
    for kind in RangeFilterKind::ALL {
      let property_type = kind.has_property_type().then_some(0x40);
      let filter = RangeFilter::new(1, property_type, &[Range::new(0, 0), Range::open(9)]).unwrap();
      roundtrip(kind, filter);
    }
  }

  #[test]
  fn a_set_id_with_no_ranges_roundtrips() {
    roundtrip(
      RangeFilterKind::ObjectId,
      RangeFilter::new(4, None, &[]).unwrap(),
    );
  }

  #[test]
  fn a_missing_property_type_is_malformed() {
    let mut wire = Bytes::from_static(&[0x00]);
    assert!(RangeFilter::deserialize(RangeFilterKind::TrackProperty, &mut wire).is_err());
  }

  #[test]
  fn an_overflowing_delta_is_rejected_when_resolved_not_when_parsed() {
    let mut wire = BytesMut::new();
    wire.put_u8(0);
    wire.put_vi(u64::MAX).unwrap();
    wire.put_vi(1).unwrap();
    let filter = RangeFilter::deserialize(RangeFilterKind::ObjectId, &mut wire.freeze()).unwrap();
    assert_eq!(filter.ranges(), Err(RangeFilterError::Overflow));
  }

  #[test]
  fn a_priority_above_255_is_rejected() {
    let filter = RangeFilter::new(0, None, &[Range::new(10, 256)]).unwrap();
    assert_eq!(
      filter.validate(RangeFilterKind::Priority),
      Err(RangeFilterError::PriorityOutOfRange { value: 256 })
    );
    // The same bounds are fine for anything wider than a byte.
    assert!(filter.validate(RangeFilterKind::ObjectId).is_ok());
    // An open end does not name a value above 255.
    let open = RangeFilter::new(0, None, &[Range::open(200)]).unwrap();
    assert!(open.validate(RangeFilterKind::Priority).is_ok());
  }

  #[test]
  fn an_odd_property_type_is_rejected() {
    let filter = RangeFilter::new(0, Some(0x3D), &[Range::new(1, 1)]).unwrap();
    for kind in [
      RangeFilterKind::ObjectProperty,
      RangeFilterKind::TrackProperty,
    ] {
      assert_eq!(
        filter.validate(kind),
        Err(RangeFilterError::OddPropertyType {
          property_type: 0x3D
        })
      );
    }
  }

  #[test]
  fn a_property_type_must_match_the_kind() {
    let with = RangeFilter::new(0, Some(0x3C), &[]).unwrap();
    let without = RangeFilter::new(0, None, &[]).unwrap();
    assert!(with.validate(RangeFilterKind::Subgroup).is_err());
    assert!(without.validate(RangeFilterKind::ObjectProperty).is_err());
  }

  #[test]
  fn descending_or_mid_list_open_ranges_cannot_be_encoded() {
    assert_eq!(
      RangeFilter::new(0, None, &[Range::new(10, 15), Range::new(3, 5)]),
      Err(RangeFilterError::Unordered)
    );
    assert_eq!(
      RangeFilter::new(0, None, &[Range::new(5, 3)]),
      Err(RangeFilterError::Unordered)
    );
    assert_eq!(
      RangeFilter::new(0, None, &[Range::open(3), Range::new(10, 15)]),
      Err(RangeFilterError::Unordered)
    );
  }

  #[test]
  fn a_range_contains_its_bounds() {
    assert!(Range::new(3, 5).contains(3));
    assert!(Range::new(3, 5).contains(5));
    assert!(!Range::new(3, 5).contains(6));
    assert!(Range::open(3).contains(u64::MAX));
    assert!(!Range::open(3).contains(2));
  }
}
