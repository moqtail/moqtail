// Copyright 2025 The MOQtail Authors
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

use crate::model::error::ParseError;
use std::convert::TryFrom;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum FetchHeaderType {
  Type0x05 = 0x05,
}

/// Subgroup Header Type.
///
/// Type bit layout (form `0b0XX1XXXX`):
/// - Bit 0 (0x01): PROPERTIES - Properties present in all objects
/// - Bits 1-2 (0x06): SUBGROUP_ID_MODE - How subgroup ID is encoded
///   - 0b00 (0x00): Subgroup ID = 0 (absent from header)
///   - 0b01 (0x02): Subgroup ID = First Object ID (absent from header)
///   - 0b10 (0x04): Subgroup ID = explicit (present in header)
///   - 0b11 (0x06): Reserved for future use (invalid, triggers PROTOCOL_VIOLATION)
/// - Bit 3 (0x08): END_OF_GROUP - This subgroup contains the final object in the group
/// - Bit 4 (0x10): Always set (distinguishes subgroup from other header types)
/// - Bit 5 (0x20): DEFAULT_PRIORITY - Publisher priority field omitted, inherited from subscription
/// - Bit 6 (0x40): FIRST_OBJECT - The first object on this stream is the first the original
///   publisher published in the subgroup
/// - Bit 7 (0x80): Must be 0
///
/// Valid Type values are exactly the ranges 0x10-0x1F, 0x30-0x3F, 0x50-0x5F and 0x70-0x7F
/// (bit 4 set, bit 7 clear), minus those with SUBGROUP_ID_MODE = 0b11: 0x16, 0x17, 0x1E, 0x1F,
/// 0x36, 0x37, 0x3E, 0x3F, 0x56, 0x57, 0x5E, 0x5F, 0x76, 0x77, 0x7E, 0x7F.
#[derive(Debug, PartialEq, Clone, Copy, Eq)]
pub struct SubgroupHeaderType(u8);

impl SubgroupHeaderType {
  /// Properties present in all objects (bit 0)
  pub const PROPERTIES: u8 = 0x01;
  /// Mask for SUBGROUP_ID_MODE (bits 1-2)
  pub const SUBGROUP_ID_MODE_MASK: u8 = 0x06;
  /// This subgroup contains the final object in the group (bit 3)
  pub const END_OF_GROUP: u8 = 0x08;
  /// Required bit that must always be set (bit 4)
  pub const REQUIRED_BIT: u8 = 0x10;
  /// Publisher priority field omitted, inherited from subscription (bit 5)
  pub const DEFAULT_PRIORITY: u8 = 0x20;
  /// First object on this stream is the first published in the subgroup (bit 6)
  pub const FIRST_OBJECT: u8 = 0x40;
  /// Mask for bits that must be zero: bit 7 only
  const INVALID_BITS_MASK: u8 = 0x80;
  /// Reserved SUBGROUP_ID_MODE value (0b11)
  const RESERVED_SUBGROUP_MODE: u8 = 0x06;

  /// Validate and create from a raw value.
  pub fn try_new(value: u64) -> Result<Self, ParseError> {
    if value > u8::MAX as u64 || (value as u8) & Self::INVALID_BITS_MASK != 0 {
      return Err(ParseError::InvalidType {
        context: "SubgroupHeaderType::try_new",
        details: format!("invalid bits set, got {value:#x}"),
      });
    }
    let v = value as u8;
    if v & Self::REQUIRED_BIT == 0 {
      return Err(ParseError::InvalidType {
        context: "SubgroupHeaderType::try_new",
        details: format!("bit 4 not set, got {value:#x}"),
      });
    }
    if v & Self::SUBGROUP_ID_MODE_MASK == Self::RESERVED_SUBGROUP_MODE {
      return Err(ParseError::InvalidType {
        context: "SubgroupHeaderType::try_new",
        details: format!("reserved SUBGROUP_ID_MODE 0b11, got {value:#x}"),
      });
    }
    Ok(Self(v))
  }

  /// Get the raw u8 value.
  pub fn value(&self) -> u8 {
    self.0
  }

  /// Check if properties are present (bit 0 set)
  pub fn has_properties(&self) -> bool {
    self.0 & Self::PROPERTIES != 0
  }

  /// Check if subgroup ID is explicit in header (SUBGROUP_ID_MODE = 0b10)
  pub fn has_explicit_subgroup_id(&self) -> bool {
    self.0 & Self::SUBGROUP_ID_MODE_MASK == 0x04
  }

  /// Check if subgroup ID = 0 (SUBGROUP_ID_MODE = 0b00)
  pub fn subgroup_id_is_zero(&self) -> bool {
    self.0 & Self::SUBGROUP_ID_MODE_MASK == 0x00
  }

  /// Check if subgroup ID = first Object ID (SUBGROUP_ID_MODE = 0b01)
  pub fn subgroup_id_is_first_object_id(&self) -> bool {
    self.0 & Self::SUBGROUP_ID_MODE_MASK == 0x02
  }

  /// Check if this subgroup contains end of group (bit 3 set)
  pub fn contains_end_of_group(&self) -> bool {
    self.0 & Self::END_OF_GROUP != 0
  }

  /// Check if using default publisher priority (bit 5 set).
  /// When true, publisher_priority field is omitted from header.
  pub fn has_default_priority(&self) -> bool {
    self.0 & Self::DEFAULT_PRIORITY != 0
  }

  /// Check if this stream's first object is the first the original publisher published
  /// in the subgroup (bit 6 set).
  pub fn is_first_object(&self) -> bool {
    self.0 & Self::FIRST_OBJECT != 0
  }

  /// Create type from property flags.
  /// subgroup_id_mode: 0=zero, 1=firstObjId, 2=explicit
  pub fn from_properties(
    has_properties: bool,
    subgroup_id_mode: u8,
    contains_end_of_group: bool,
    has_default_priority: bool,
    first_object: bool,
  ) -> Self {
    let mut v: u8 = Self::REQUIRED_BIT;
    if has_properties {
      v |= Self::PROPERTIES;
    }
    v |= (subgroup_id_mode & 0x03) << 1; // bits 1-2
    if contains_end_of_group {
      v |= Self::END_OF_GROUP;
    }
    if has_default_priority {
      v |= Self::DEFAULT_PRIORITY;
    }
    if first_object {
      v |= Self::FIRST_OBJECT;
    }
    Self(v)
  }
}

impl TryFrom<u64> for SubgroupHeaderType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    Self::try_new(value)
  }
}

impl From<SubgroupHeaderType> for u64 {
  fn from(t: SubgroupHeaderType) -> Self {
    t.0 as u64
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectForwardingPreference {
  Subgroup,
  Datagram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ObjectStatus {
  /// 0x0: Normal object. Implicit for any non-zero length object. Zero-length objects explicitly encode this status.
  Normal = 0x0,
  /// 0x3: Indicates end of Group. ObjectId is one greater than the largest object produced in the group identified by the GroupID.
  /// Sent right after the last object in the group. If ObjectID is 0, there are no Objects in this Group. SHOULD be cached.
  EndOfGroup = 0x3,
  /// 0x4: Indicates end of Track. GroupID is either the largest group produced in this track and the ObjectID is one greater than the largest object produced in that group,
  /// or GroupID is one greater than the largest group produced in this track and the ObjectID is zero. SHOULD be cached.
  EndOfTrack = 0x4,
}

impl TryFrom<u64> for ObjectStatus {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(ObjectStatus::Normal),
      0x3 => Ok(ObjectStatus::EndOfGroup),
      0x4 => Ok(ObjectStatus::EndOfTrack),
      _ => Err(ParseError::InvalidType {
        context: "ObjectStatus::try_from(u8)",
        details: format!("Invalid status, got {value}"),
      }),
    }
  }
}

impl From<ObjectStatus> for u64 {
  fn from(status: ObjectStatus) -> Self {
    status as u64
  }
}

/// Draft-16 Object Datagram Type (bitmask newtype).
///
/// Type bit layout (form `0b00X0XXXX`):
/// - Bit 0 (0x01): PROPERTIES — Properties field present
/// - Bit 1 (0x02): END_OF_GROUP — Last object in group
/// - Bit 2 (0x04): ZERO_OBJECT_ID — Object ID omitted (assumed 0)
/// - Bit 3 (0x08): DEFAULT_PRIORITY — Publisher Priority omitted (inherited)
/// - Bit 5 (0x20): STATUS — Object Status replaces Object Payload
///
/// Invalid combinations:
/// - STATUS (0x20) + END_OF_GROUP (0x02) together → PROTOCOL_VIOLATION
/// - Types outside the form `0b00X0XXXX` (bit 4, 6, 7 must be 0) → PROTOCOL_VIOLATION
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectDatagramType(u8);

impl ObjectDatagramType {
  pub const PROPERTIES: u8 = 0x01;
  pub const END_OF_GROUP: u8 = 0x02;
  pub const ZERO_OBJECT_ID: u8 = 0x04;
  pub const DEFAULT_PRIORITY: u8 = 0x08;
  pub const STATUS: u8 = 0x20;

  /// Mask for bits that must be zero: bits 4, 6, 7 (form `0b00X0XXXX`).
  const INVALID_BITS_MASK: u8 = 0xD0;

  /// Validate and create from a raw value.
  pub fn try_new(value: u64) -> Result<Self, ParseError> {
    if value > u8::MAX as u64 || (value as u8) & Self::INVALID_BITS_MASK != 0 {
      return Err(ParseError::InvalidType {
        context: "ObjectDatagramType::try_new",
        details: format!("Invalid datagram type {value:#x}, must match form 0b00X0XXXX"),
      });
    }
    let v = value as u8;
    if v & Self::STATUS != 0 && v & Self::END_OF_GROUP != 0 {
      return Err(ParseError::ProtocolViolation {
        context: "ObjectDatagramType::try_new",
        details: "STATUS and END_OF_GROUP cannot both be set".to_string(),
      });
    }
    Ok(Self(v))
  }

  /// Get the raw u8 value.
  pub fn value(&self) -> u8 {
    self.0
  }

  /// Check if properties are present (bit 0).
  pub fn has_properties(&self) -> bool {
    self.0 & Self::PROPERTIES != 0
  }

  /// Check if this is end of group (bit 1).
  pub fn is_end_of_group(&self) -> bool {
    self.0 & Self::END_OF_GROUP != 0
  }

  /// Check if Object ID is absent (bit 2 set).
  /// When true, Object ID is omitted and assumed to be 0.
  pub fn is_zero_object_id(&self) -> bool {
    self.0 & Self::ZERO_OBJECT_ID != 0
  }

  /// Check if Publisher Priority is omitted (bit 3 set).
  /// When true, the priority is inherited from the control message.
  pub fn has_default_priority(&self) -> bool {
    self.0 & Self::DEFAULT_PRIORITY != 0
  }

  /// Check if the datagram carries Object Status instead of payload (bit 5 set).
  pub fn is_status(&self) -> bool {
    self.0 & Self::STATUS != 0
  }

  /// Create type from property flags.
  /// Returns error if STATUS and END_OF_GROUP are both true.
  pub fn from_properties(
    has_properties: bool,
    end_of_group: bool,
    object_id_is_zero: bool,
    default_priority: bool,
    is_status: bool,
  ) -> Result<Self, ParseError> {
    let mut v: u8 = 0;
    if has_properties {
      v |= Self::PROPERTIES;
    }
    if end_of_group {
      v |= Self::END_OF_GROUP;
    }
    if object_id_is_zero {
      v |= Self::ZERO_OBJECT_ID;
    }
    if default_priority {
      v |= Self::DEFAULT_PRIORITY;
    }
    if is_status {
      v |= Self::STATUS;
    }
    Self::try_new(v as u64)
  }
}

impl TryFrom<u64> for ObjectDatagramType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    Self::try_new(value)
  }
}

impl From<ObjectDatagramType> for u64 {
  fn from(dtype: ObjectDatagramType) -> Self {
    dtype.0 as u64
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The independent oracle for a valid SUBGROUP_HEADER type byte:
  /// bit 7 clear, bit 4 set, and SUBGROUP_ID_MODE (bits 1-2) not 0b11.
  fn is_valid_subgroup_type(b: u8) -> bool {
    b & 0x80 == 0 && b & 0x10 != 0 && b & 0x06 != 0x06
  }

  #[test]
  fn subgroup_header_type_classifies_all_256_bytes() {
    // The 16 reserved SUBGROUP_ID_MODE = 0b11 values named explicitly, as a
    // cross-check that the oracle and the spec agree.
    let reserved_mode_3 = [
      0x16u8, 0x17, 0x1E, 0x1F, 0x36, 0x37, 0x3E, 0x3F, 0x56, 0x57, 0x5E, 0x5F, 0x76, 0x77, 0x7E,
      0x7F,
    ];

    for b in 0u8..=255 {
      let accepted = SubgroupHeaderType::try_new(b as u64).is_ok();
      let expected = is_valid_subgroup_type(b);
      assert_eq!(accepted, expected, "type byte {b:#04x}");

      if reserved_mode_3.contains(&b) {
        assert!(
          !accepted,
          "reserved SUBGROUP_ID_MODE 0b11 value {b:#04x} must be rejected"
        );
      }
      // Round-trips preserve the byte for accepted values.
      if accepted {
        assert_eq!(SubgroupHeaderType::try_new(b as u64).unwrap().value(), b);
      }
    }

    // Exactly the four ranges minus the 16 reserved values are valid.
    let valid_count = (0u8..=255).filter(|&b| is_valid_subgroup_type(b)).count();
    assert_eq!(valid_count, 4 * 16 - 16, "expected 48 valid type bytes");
  }

  #[test]
  fn first_object_bit_round_trips() {
    let with = SubgroupHeaderType::from_properties(false, 2, false, false, true);
    assert!(with.is_first_object());
    assert_eq!(with.value() & SubgroupHeaderType::FIRST_OBJECT, 0x40);
    assert_eq!(
      SubgroupHeaderType::try_new(with.value() as u64).unwrap(),
      with
    );

    let without = SubgroupHeaderType::from_properties(false, 2, false, false, false);
    assert!(!without.is_first_object());
  }

  #[test]
  fn first_object_alone_is_a_valid_type() {
    // 0x50 = FIRST_OBJECT (0x40) + REQUIRED_BIT (0x10), mode 0b00. Rejected before RS-14
    // because 0x40 was in INVALID_BITS_MASK.
    let t = SubgroupHeaderType::try_new(0x50).unwrap();
    assert!(t.is_first_object());
  }
}
