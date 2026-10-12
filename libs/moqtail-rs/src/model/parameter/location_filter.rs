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

use crate::model::common::location::Location;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::control::constant::LocationFilterType;
use crate::model::error::ParseError;
use bytes::{Bytes, BytesMut};

/// The range of Locations a subscription or fetch selects.
///
/// Every Group here is absolute. The wire carries the End Group as a delta from
/// the Start Group, but that is an encoding detail: [`Self::deserialize`] adds it
/// back and [`Self::serialize`] takes it off again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationFilter {
  /// No filter: every Object passes.
  None,
  /// Starts `start_group` groups before the Next Group, at
  /// `{Largest Object.Group + 1 - start_group, 0}`. 0 is the Next Group, 1 the
  /// current one. Open-ended.
  RelativeStart { start_group: u64 },
  /// Open-ended.
  AbsoluteStart { start: Location },
  /// Ends with the last Object of `end_group`.
  AbsoluteStartGroupEnd { start: Location, end_group: u64 },
  /// Ends with `end`, inclusive.
  AbsoluteRange { start: Location, end: Location },
  /// Starts at the Object after the Largest Object. Open-ended.
  NextObject,
}

impl LocationFilter {
  pub fn filter_type(&self) -> LocationFilterType {
    match self {
      Self::None => LocationFilterType::None,
      Self::RelativeStart { .. } => LocationFilterType::RelativeStart,
      Self::AbsoluteStart { .. } => LocationFilterType::AbsoluteStart,
      Self::AbsoluteStartGroupEnd { .. } => LocationFilterType::AbsoluteStartGroupEnd,
      Self::AbsoluteRange { .. } => LocationFilterType::AbsoluteRange,
      Self::NextObject => LocationFilterType::NextObject,
    }
  }

  /// The first Location the filter passes, given the track's Largest Object, or
  /// `None` when there is no filter.
  ///
  /// A relative start that would fall before group 0 is group 0. A track with
  /// nothing published yet starts a relative filter at `{0,0}`, which passes
  /// whatever comes first.
  pub fn start_location(&self, largest: Option<Location>) -> Option<Location> {
    match self {
      Self::None => None,
      Self::RelativeStart { start_group } => Some(match largest {
        Some(largest) => {
          // Computed wide so neither the +1 nor the subtraction can wrap.
          let group = (largest.group as u128 + 1).saturating_sub(*start_group as u128);
          Location::new(group.min(u64::MAX as u128) as u64, 0)
        }
        None => Location::new(0, 0),
      }),
      Self::AbsoluteStart { start }
      | Self::AbsoluteStartGroupEnd { start, .. }
      | Self::AbsoluteRange { start, .. } => Some(start.clone()),
      Self::NextObject => Some(match largest {
        Some(largest) => Location::new(largest.group, largest.object.saturating_add(1)),
        None => Location::new(0, 0),
      }),
    }
  }

  /// The last Location the filter passes, inclusive, or `None` when it is
  /// open-ended. A Group End is the whole of that group, so its Object is
  /// `u64::MAX`.
  pub fn end_location(&self) -> Option<Location> {
    match self {
      Self::AbsoluteStartGroupEnd { end_group, .. } => Some(Location::new(*end_group, u64::MAX)),
      Self::AbsoluteRange { end, .. } => Some(end.clone()),
      _ => None,
    }
  }

  /// The inclusive range the filter selects under the rules for a Fetch, which
  /// never reach past the Largest Object: an open end is the Largest Object, and
  /// no filter at all is the whole track up to it.
  ///
  /// `None` when that range is empty, which includes a track with nothing
  /// published yet and a Next Object filter, whose start is always past the
  /// Largest Object.
  pub fn fetch_range(&self, largest: Option<Location>) -> Option<(Location, Location)> {
    let largest = largest?;
    let start = self
      .start_location(Some(largest.clone()))
      .unwrap_or_default();
    let end = match self.end_location() {
      Some(end) if end < largest => end,
      _ => largest,
    };
    (start <= end).then_some((start, end))
  }

  /// Encodes the Location Filter Type and the fields it calls for. The Parameter
  /// Type is not included.
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(self.filter_type() as u64)?;
    match self {
      Self::None | Self::NextObject => {}
      Self::RelativeStart { start_group } => buf.put_vi(*start_group)?,
      Self::AbsoluteStart { start } => {
        buf.put_vi(start.group)?;
        buf.put_vi(start.object)?;
      }
      Self::AbsoluteStartGroupEnd { start, end_group } => {
        buf.put_vi(start.group)?;
        buf.put_vi(start.object)?;
        buf.put_vi(end_group_delta(start.group, *end_group)?)?;
      }
      Self::AbsoluteRange { start, end } => {
        buf.put_vi(start.group)?;
        buf.put_vi(start.object)?;
        buf.put_vi(end_group_delta(start.group, end.group)?)?;
        buf.put_vi(end.object)?;
      }
    }
    Ok(buf.freeze())
  }

  /// Decodes a Location Filter Type and the fields it calls for. The Parameter
  /// Type has already been read.
  ///
  /// An unknown Location Filter Type, and an End Group past 2^64 - 1, are a
  /// PROTOCOL_VIOLATION.
  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let raw = bytes.get_vi()?;
    let filter_type =
      LocationFilterType::try_from(raw).map_err(|_| ParseError::ProtocolViolation {
        context: "LocationFilter::deserialize",
        details: format!("unknown Location Filter Type {raw:#x}"),
      })?;
    Ok(match filter_type {
      LocationFilterType::None => Self::None,
      LocationFilterType::RelativeStart => Self::RelativeStart {
        start_group: bytes.get_vi()?,
      },
      LocationFilterType::AbsoluteStart => Self::AbsoluteStart {
        start: Location::deserialize(bytes)?,
      },
      LocationFilterType::AbsoluteStartGroupEnd => {
        let start = Location::deserialize(bytes)?;
        let end_group = end_group(start.group, bytes.get_vi()?)?;
        Self::AbsoluteStartGroupEnd { start, end_group }
      }
      LocationFilterType::AbsoluteRange => {
        let start = Location::deserialize(bytes)?;
        let end_group = end_group(start.group, bytes.get_vi()?)?;
        let end = Location::new(end_group, bytes.get_vi()?);
        Self::AbsoluteRange { start, end }
      }
      LocationFilterType::NextObject => Self::NextObject,
    })
  }
}

fn end_group(start_group: u64, delta: u64) -> Result<u64, ParseError> {
  start_group
    .checked_add(delta)
    .ok_or_else(|| ParseError::ProtocolViolation {
      context: "LocationFilter::deserialize",
      details: format!("StartGroup {start_group} plus EndGroupDelta {delta} exceeds 2^64 - 1"),
    })
}

/// A delta is unsigned, so an End Group before the Start Group has no encoding.
fn end_group_delta(start_group: u64, end_group: u64) -> Result<u64, ParseError> {
  end_group
    .checked_sub(start_group)
    .ok_or_else(|| ParseError::ProtocolViolation {
      context: "LocationFilter::serialize",
      details: format!("End Group {end_group} is before Start Group {start_group}"),
    })
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  fn loc(group: u64, object: u64) -> Location {
    Location::new(group, object)
  }

  fn roundtrip(filter: LocationFilter) {
    let mut bytes = filter.serialize().unwrap();
    let decoded = LocationFilter::deserialize(&mut bytes).unwrap();
    assert_eq!(decoded, filter);
    assert!(!bytes.has_remaining(), "{filter:?} left bytes behind");
  }

  #[test]
  fn every_filter_type_roundtrips() {
    roundtrip(LocationFilter::None);
    roundtrip(LocationFilter::RelativeStart { start_group: 3 });
    roundtrip(LocationFilter::AbsoluteStart { start: loc(10, 2) });
    roundtrip(LocationFilter::AbsoluteStartGroupEnd {
      start: loc(10, 2),
      end_group: 15,
    });
    roundtrip(LocationFilter::AbsoluteRange {
      start: loc(10, 2),
      end: loc(15, 7),
    });
    roundtrip(LocationFilter::NextObject);
  }

  #[test]
  fn the_type_decides_which_fields_follow() {
    let cases: [(LocationFilter, &[u8]); 6] = [
      (LocationFilter::None, &[0x00]),
      (
        LocationFilter::RelativeStart { start_group: 1 },
        &[0x01, 0x01],
      ),
      (
        LocationFilter::AbsoluteStart { start: loc(5, 2) },
        &[0x02, 0x05, 0x02],
      ),
      (
        LocationFilter::AbsoluteStartGroupEnd {
          start: loc(5, 2),
          end_group: 8,
        },
        &[0x03, 0x05, 0x02, 0x03],
      ),
      (
        LocationFilter::AbsoluteRange {
          start: loc(5, 2),
          end: loc(8, 4),
        },
        &[0x04, 0x05, 0x02, 0x03, 0x04],
      ),
      (LocationFilter::NextObject, &[0x05]),
    ];
    for (filter, wire) in cases {
      assert_eq!(filter.serialize().unwrap().as_ref(), wire, "{filter:?}");
    }
  }

  #[test]
  fn the_end_group_is_a_delta_from_the_start_group_on_the_wire() {
    let mut bytes = Bytes::from_static(&[0x03, 0x0A, 0x00, 0x00]);
    assert_eq!(
      LocationFilter::deserialize(&mut bytes).unwrap(),
      LocationFilter::AbsoluteStartGroupEnd {
        start: loc(10, 0),
        end_group: 10,
      }
    );
  }

  #[test]
  fn decoding_reads_only_the_fields_its_type_calls_for() {
    // A Next Object filter followed by the next parameter's bytes.
    let mut bytes = Bytes::from_static(&[0x05, 0x10, 0x01]);
    assert_eq!(
      LocationFilter::deserialize(&mut bytes).unwrap(),
      LocationFilter::NextObject
    );
    assert_eq!(bytes.as_ref(), &[0x10, 0x01]);
  }

  #[test]
  fn an_unknown_filter_type_is_a_protocol_violation() {
    let mut bytes = Bytes::from_static(&[0x06]);
    assert!(matches!(
      LocationFilter::deserialize(&mut bytes),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn an_end_group_past_the_largest_group_id_is_a_protocol_violation() {
    let mut buf = BytesMut::new();
    buf
      .put_vi(LocationFilterType::AbsoluteStartGroupEnd as u64)
      .unwrap();
    buf.put_vi(u64::MAX >> 2).unwrap();
    buf.put_vi(0).unwrap();
    buf.put_vi(u64::MAX >> 2).unwrap();
    // StartGroup + EndGroupDelta fits: the varint maximum is 2^62 - 1.
    let mut bytes = buf.freeze();
    assert!(LocationFilter::deserialize(&mut bytes).is_ok());

    // The sum can only overflow through a value that cannot be a varint, so the
    // check is exercised directly.
    assert!(matches!(
      end_group(u64::MAX, 1),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn a_truncated_filter_is_not_enough_bytes() {
    let mut bytes = Bytes::from_static(&[0x04, 0x05, 0x02, 0x03]);
    assert!(matches!(
      LocationFilter::deserialize(&mut bytes),
      Err(ParseError::NotEnoughBytes { .. })
    ));
  }

  #[test]
  fn an_end_group_before_the_start_group_cannot_be_encoded() {
    let filter = LocationFilter::AbsoluteStartGroupEnd {
      start: loc(10, 0),
      end_group: 9,
    };
    assert!(matches!(
      filter.serialize(),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn a_relative_start_counts_back_from_the_next_group() {
    let largest = Some(loc(7, 3));
    let start =
      |start_group| LocationFilter::RelativeStart { start_group }.start_location(largest.clone());
    assert_eq!(start(0), Some(loc(8, 0)), "the Next Group");
    assert_eq!(start(1), Some(loc(7, 0)), "the current group");
    assert_eq!(start(2), Some(loc(6, 0)));
    assert_eq!(start(100), Some(loc(0, 0)), "clamped at group 0");
  }

  #[test]
  fn a_relative_start_from_the_last_group_id_does_not_wrap() {
    let filter = LocationFilter::RelativeStart { start_group: 0 };
    assert_eq!(
      filter.start_location(Some(loc(u64::MAX, 0))),
      Some(loc(u64::MAX, 0))
    );
  }

  #[test]
  fn next_object_starts_after_the_largest_object() {
    assert_eq!(
      LocationFilter::NextObject.start_location(Some(loc(7, 3))),
      Some(loc(7, 4))
    );
  }

  #[test]
  fn a_track_with_nothing_published_starts_a_relative_filter_at_the_beginning() {
    for filter in [
      LocationFilter::NextObject,
      LocationFilter::RelativeStart { start_group: 0 },
      LocationFilter::RelativeStart { start_group: 5 },
    ] {
      assert_eq!(filter.start_location(None), Some(loc(0, 0)), "{filter:?}");
    }
  }

  #[test]
  fn no_filter_has_no_start() {
    assert_eq!(LocationFilter::None.start_location(Some(loc(7, 3))), None);
  }

  #[test]
  fn a_fetch_range_ends_at_the_largest_object_when_open_ended() {
    let largest = Some(loc(7, 3));
    assert_eq!(
      LocationFilter::None.fetch_range(largest.clone()),
      Some((loc(0, 0), loc(7, 3)))
    );
    assert_eq!(
      LocationFilter::AbsoluteStart { start: loc(5, 1) }.fetch_range(largest.clone()),
      Some((loc(5, 1), loc(7, 3)))
    );
    assert_eq!(
      LocationFilter::RelativeStart { start_group: 1 }.fetch_range(largest),
      Some((loc(7, 0), loc(7, 3)))
    );
  }

  #[test]
  fn a_fetch_range_keeps_an_end_before_the_largest_object() {
    let largest = Some(loc(7, 3));
    assert_eq!(
      LocationFilter::AbsoluteStartGroupEnd {
        start: loc(2, 0),
        end_group: 4,
      }
      .fetch_range(largest.clone()),
      Some((loc(2, 0), loc(4, u64::MAX)))
    );
    assert_eq!(
      LocationFilter::AbsoluteRange {
        start: loc(2, 0),
        end: loc(4, 6),
      }
      .fetch_range(largest.clone()),
      Some((loc(2, 0), loc(4, 6)))
    );
    // An end past it is cut back to it.
    assert_eq!(
      LocationFilter::AbsoluteRange {
        start: loc(2, 0),
        end: loc(9, 0),
      }
      .fetch_range(largest),
      Some((loc(2, 0), loc(7, 3)))
    );
  }

  #[test]
  fn a_fetch_range_is_empty_when_nothing_precedes_it() {
    assert_eq!(
      LocationFilter::NextObject.fetch_range(Some(loc(7, 3))),
      None
    );
    assert_eq!(
      LocationFilter::RelativeStart { start_group: 0 }.fetch_range(Some(loc(7, 3))),
      None
    );
    assert_eq!(
      LocationFilter::AbsoluteStart { start: loc(8, 0) }.fetch_range(Some(loc(7, 3))),
      None
    );
    assert_eq!(LocationFilter::None.fetch_range(None), None);
  }
}
