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

/// Subgroup Header Type (Draft-16)
///
/// Type bit layout (0b00X1XXXX):
/// - Bit 0 (0x01): EXTENSIONS - Extensions present in all objects
/// - Bits 1-2 (0x06): SUBGROUP_ID_MODE - How subgroup ID is encoded
///   - 0b00 (0x00): Subgroup ID = 0 (absent from header)
///   - 0b01 (0x02): Subgroup ID = First Object ID (absent from header)
///   - 0b10 (0x04): Subgroup ID = explicit (present in header)
///   - 0b11 (0x06): Reserved for future use (invalid, triggers PROTOCOL_VIOLATION)
/// - Bit 3 (0x08): END_OF_GROUP - This subgroup contains the final object in the group
/// - Bit 4 (0x10): Always set (distinguishes subgroup from other header types)
/// - Bit 5 (0x20): DEFAULT_PRIORITY - Publisher priority field omitted, inherited from subscription
///
/// Valid ranges: 0x10-0x15, 0x18-0x1D (bit 5=0), 0x30-0x35, 0x38-0x3D (bit 5=1)
/// Invalid: 0x16, 0x17, 0x1E, 0x1F, 0x36, 0x37, 0x3E, 0x3F (SUBGROUP_ID_MODE=0b11)
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SubgroupHeaderType {
  // Bit 5 = 0 (priority present)
  /// Subgroup ID = 0, No Extensions, No End of Group (0x10)
  Type0x10 = 0x10,
  /// Subgroup ID = 0, Extensions Present, No End of Group (0x11)
  Type0x11 = 0x11,
  /// Subgroup ID = First Object ID, No Extensions, No End of Group (0x12)
  Type0x12 = 0x12,
  /// Subgroup ID = First Object ID, Extensions Present, No End of Group (0x13)
  Type0x13 = 0x13,
  /// Explicit Subgroup ID, No Extensions, No End of Group (0x14)
  Type0x14 = 0x14,
  /// Explicit Subgroup ID, Extensions Present, No End of Group (0x15)
  Type0x15 = 0x15,
  /// Subgroup ID = 0, No Extensions, Contains End of Group (0x18)
  Type0x18 = 0x18,
  /// Subgroup ID = 0, Extensions Present, Contains End of Group (0x19)
  Type0x19 = 0x19,
  /// Subgroup ID = First Object ID, No Extensions, Contains End of Group (0x1A)
  Type0x1A = 0x1A,
  /// Subgroup ID = First Object ID, Extensions Present, Contains End of Group (0x1B)
  Type0x1B = 0x1B,
  /// Explicit Subgroup ID, No Extensions, Contains End of Group (0x1C)
  Type0x1C = 0x1C,
  /// Explicit Subgroup ID, Extensions Present, Contains End of Group (0x1D)
  Type0x1D = 0x1D,
  // Bit 5 = 1 (default priority)
  /// Subgroup ID = 0, No Extensions, No End of Group, Default Priority (0x30)
  Type0x30 = 0x30,
  /// Subgroup ID = 0, Extensions Present, No End of Group, Default Priority (0x31)
  Type0x31 = 0x31,
  /// Subgroup ID = First Object ID, No Extensions, No End of Group, Default Priority (0x32)
  Type0x32 = 0x32,
  /// Subgroup ID = First Object ID, Extensions Present, No End of Group, Default Priority (0x33)
  Type0x33 = 0x33,
  /// Explicit Subgroup ID, No Extensions, No End of Group, Default Priority (0x34)
  Type0x34 = 0x34,
  /// Explicit Subgroup ID, Extensions Present, No End of Group, Default Priority (0x35)
  Type0x35 = 0x35,
  /// Subgroup ID = 0, No Extensions, Contains End of Group, Default Priority (0x38)
  Type0x38 = 0x38,
  /// Subgroup ID = 0, Extensions Present, Contains End of Group, Default Priority (0x39)
  Type0x39 = 0x39,
  /// Subgroup ID = First Object ID, No Extensions, Contains End of Group, Default Priority (0x3A)
  Type0x3A = 0x3A,
  /// Subgroup ID = First Object ID, Extensions Present, Contains End of Group, Default Priority (0x3B)
  Type0x3B = 0x3B,
  /// Explicit Subgroup ID, No Extensions, Contains End of Group, Default Priority (0x3C)
  Type0x3C = 0x3C,
  /// Explicit Subgroup ID, Extensions Present, Contains End of Group, Default Priority (0x3D)
  Type0x3D = 0x3D,
}

impl SubgroupHeaderType {
  /// Check if extensions are present (bit 0 set)
  pub fn has_extensions(&self) -> bool {
    (*self as u64) & 0x01 != 0
  }

  /// Check if subgroup ID is explicit in header (bits 1-2 = 0b10)
  pub fn has_explicit_subgroup_id(&self) -> bool {
    (*self as u64) & 0x06 == 0x04
  }

  /// Check if subgroup ID = 0 (bits 1-2 = 0b00)
  pub fn subgroup_id_is_zero(&self) -> bool {
    (*self as u64) & 0x06 == 0x00
  }

  /// Check if subgroup ID = first Object ID (bits 1-2 = 0b01)
  pub fn subgroup_id_is_first_object_id(&self) -> bool {
    (*self as u64) & 0x06 == 0x02
  }

  /// Check if this subgroup contains end of group (bit 3 set)
  pub fn contains_end_of_group(&self) -> bool {
    (*self as u64) & 0x08 != 0
  }

  /// Check if using default publisher priority (bit 5 set).
  /// When true, publisher_priority field is omitted from header.
  pub fn has_default_priority(&self) -> bool {
    (*self as u64) & 0x20 != 0
  }

  /// Create type from properties.
  /// subgroup_id_mode: 0=zero, 1=firstObjId, 2=explicit
  pub fn from_properties(
    has_extensions: bool,
    subgroup_id_mode: u64,
    contains_end_of_group: bool,
    has_default_priority: bool,
  ) -> Self {
    let mut type_val: u64 = 0x10; // bit 4 always set
    if has_extensions {
      type_val |= 0x01;
    }
    type_val |= (subgroup_id_mode & 0x03) << 1; // bits 1-2
    if contains_end_of_group {
      type_val |= 0x08;
    }
    if has_default_priority {
      type_val |= 0x20;
    }
    // Safe because we construct valid patterns
    Self::try_from(type_val).unwrap()
  }
}

impl TryFrom<u64> for SubgroupHeaderType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    // Validate bit layout before matching
    // Bit 4 (0x10) must be set to distinguish from other header types
    if (value & 0x10) == 0 {
      return Err(ParseError::InvalidType {
        context: "SubgroupHeaderType::try_from(u64)",
        details: format!("bit 4 not set, got {value:#x}"),
      });
    }

    // SUBGROUP_ID_MODE (bits 1-2) must not be 0b11 (reserved for future use)
    if (value & 0x06) == 0x06 {
      return Err(ParseError::InvalidType {
        context: "SubgroupHeaderType::try_from(u64)",
        details: format!("reserved SUBGROUP_ID_MODE 0b11, got {value:#x}"),
      });
    }

    // Must be in valid range [0x10, 0x3F]
    if !(0x10..=0x3F).contains(&value) {
      return Err(ParseError::InvalidType {
        context: "SubgroupHeaderType::try_from(u64)",
        details: format!("out of valid range, got {value:#x}"),
      });
    }

    match value {
      // Bit 5 = 0 (priority present)
      0x10 => Ok(SubgroupHeaderType::Type0x10),
      0x11 => Ok(SubgroupHeaderType::Type0x11),
      0x12 => Ok(SubgroupHeaderType::Type0x12),
      0x13 => Ok(SubgroupHeaderType::Type0x13),
      0x14 => Ok(SubgroupHeaderType::Type0x14),
      0x15 => Ok(SubgroupHeaderType::Type0x15),
      0x18 => Ok(SubgroupHeaderType::Type0x18),
      0x19 => Ok(SubgroupHeaderType::Type0x19),
      0x1A => Ok(SubgroupHeaderType::Type0x1A),
      0x1B => Ok(SubgroupHeaderType::Type0x1B),
      0x1C => Ok(SubgroupHeaderType::Type0x1C),
      0x1D => Ok(SubgroupHeaderType::Type0x1D),
      // Bit 5 = 1 (default priority)
      0x30 => Ok(SubgroupHeaderType::Type0x30),
      0x31 => Ok(SubgroupHeaderType::Type0x31),
      0x32 => Ok(SubgroupHeaderType::Type0x32),
      0x33 => Ok(SubgroupHeaderType::Type0x33),
      0x34 => Ok(SubgroupHeaderType::Type0x34),
      0x35 => Ok(SubgroupHeaderType::Type0x35),
      0x38 => Ok(SubgroupHeaderType::Type0x38),
      0x39 => Ok(SubgroupHeaderType::Type0x39),
      0x3A => Ok(SubgroupHeaderType::Type0x3A),
      0x3B => Ok(SubgroupHeaderType::Type0x3B),
      0x3C => Ok(SubgroupHeaderType::Type0x3C),
      0x3D => Ok(SubgroupHeaderType::Type0x3D),
      _ => Err(ParseError::InvalidType {
        context: "SubgroupHeaderType::try_from(u64)",
        details: format!("Invalid type value, got {value:#x}"),
      }),
    }
  }
}

impl From<SubgroupHeaderType> for u64 {
  fn from(header_type: SubgroupHeaderType) -> Self {
    header_type as u64
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
  /// 0x1: Indicates Object does not exist. This object does not exist at any publisher and will not be published in the future. SHOULD be cached.
  DoesNotExist = 0x1,
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
      0x1 => Ok(ObjectStatus::DoesNotExist),
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

/// Draft-14 Object Datagram Type (0x00-0x07)
///
/// Type encoding uses bit flags:
/// - Bit 0: Extensions Present (0 = no, 1 = yes)
/// - Bit 1: End of Group (0 = no, 1 = yes)
/// - Bit 2: Object ID Absent (0 = present, 1 = absent when Object ID = 0)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ObjectDatagramType {
  /// 0x00: No Extensions, Not End of Group, Object ID Present
  Type0x00 = 0x00,
  /// 0x01: With Extensions, Not End of Group, Object ID Present
  Type0x01 = 0x01,
  /// 0x02: No Extensions, End of Group, Object ID Present
  Type0x02 = 0x02,
  /// 0x03: With Extensions, End of Group, Object ID Present
  Type0x03 = 0x03,
  /// 0x04: No Extensions, Not End of Group, Object ID Absent (Object ID = 0)
  Type0x04 = 0x04,
  /// 0x05: With Extensions, Not End of Group, Object ID Absent (Object ID = 0)
  Type0x05 = 0x05,
  /// 0x06: No Extensions, End of Group, Object ID Absent (Object ID = 0)
  Type0x06 = 0x06,
  /// 0x07: With Extensions, End of Group, Object ID Absent (Object ID = 0)
  Type0x07 = 0x07,
}

impl ObjectDatagramType {
  /// Check if extensions are present (bit 0)
  pub fn has_extensions(&self) -> bool {
    (*self as u64) & 0x01 != 0
  }

  /// Check if this is end of group (bit 1)
  pub fn is_end_of_group(&self) -> bool {
    (*self as u64) & 0x02 != 0
  }

  /// Check if Object ID field is present (inverted bit 2)
  pub fn has_object_id(&self) -> bool {
    (*self as u64) & 0x04 == 0
  }

  /// Create type from properties
  pub fn from_properties(
    has_extensions: bool,
    end_of_group: bool,
    object_id_is_zero: bool,
  ) -> Self {
    let mut type_val: u64 = 0;
    if has_extensions {
      type_val |= 0x01;
    }
    if end_of_group {
      type_val |= 0x02;
    }
    if object_id_is_zero {
      type_val |= 0x04;
    }
    // Safe because we only set bits 0-2, resulting in 0x00-0x07
    Self::try_from(type_val).unwrap()
  }
}

impl TryFrom<u64> for ObjectDatagramType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x00 => Ok(ObjectDatagramType::Type0x00),
      0x01 => Ok(ObjectDatagramType::Type0x01),
      0x02 => Ok(ObjectDatagramType::Type0x02),
      0x03 => Ok(ObjectDatagramType::Type0x03),
      0x04 => Ok(ObjectDatagramType::Type0x04),
      0x05 => Ok(ObjectDatagramType::Type0x05),
      0x06 => Ok(ObjectDatagramType::Type0x06),
      0x07 => Ok(ObjectDatagramType::Type0x07),
      _ => Err(ParseError::InvalidType {
        context: "ObjectDatagramType::try_from(u64)",
        details: format!("Invalid type, expected 0x00-0x07, got {value:#x}"),
      }),
    }
  }
}

impl From<ObjectDatagramType> for u64 {
  fn from(dtype: ObjectDatagramType) -> Self {
    dtype as u64
  }
}

/// Draft-14 Object Datagram Status Type (0x20-0x21)
///
/// Status datagrams always have Object ID present.
/// - 0x20: Without Extensions
/// - 0x21: With Extensions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ObjectDatagramStatusType {
  /// 0x20: Without Extensions, Object ID Present
  WithoutExtensions = 0x20,
  /// 0x21: With Extensions, Object ID Present
  WithExtensions = 0x21,
}

impl ObjectDatagramStatusType {
  /// Check if extensions are present
  pub fn has_extensions(&self) -> bool {
    *self == ObjectDatagramStatusType::WithExtensions
  }
}

impl TryFrom<u64> for ObjectDatagramStatusType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x20 => Ok(ObjectDatagramStatusType::WithoutExtensions),
      0x21 => Ok(ObjectDatagramStatusType::WithExtensions),
      _ => Err(ParseError::InvalidType {
        context: "ObjectDatagramStatusType::try_from(u64)",
        details: format!("Invalid type, expected 0x20 or 0x21, got {value:#x}"),
      }),
    }
  }
}

impl From<ObjectDatagramStatusType> for u64 {
  fn from(dtype: ObjectDatagramStatusType) -> Self {
    dtype as u64
  }
}

/// Fetch Object Serialization Flags (Draft-16 §10.4.4.1)
///
/// Bitmask field controlling the wire format of Fetch Objects.
/// Valid values: 0x00–0x7F, 0x8C (End of Non-Existent Range), 0x10C (End of Unknown Range).
/// Any other value >= 128 is a PROTOCOL_VIOLATION.
///
/// Bit layout (values < 128):
/// - bits 1-0 (0x03): SubgroupIdMode
///   - 0x00: Subgroup ID is zero
///   - 0x01: Subgroup ID is prior object's Subgroup ID
///   - 0x02: Subgroup ID is prior object's Subgroup ID + 1
///   - 0x03: Subgroup ID field is explicit (present in wire format)
/// - bit 2 (0x04): Object ID field explicit  (0 = prior object's ID + 1)
/// - bit 3 (0x08): Group ID field explicit   (0 = same as prior object)
/// - bit 4 (0x10): Publisher Priority explicit (0 = same as prior object)
/// - bit 5 (0x20): Extensions field present
/// - bit 6 (0x40): Datagram mode (bits 0-1 ignored, no Subgroup ID field)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchObjectSerializationFlags(pub u64);

impl FetchObjectSerializationFlags {
  /// End of Non-Existent Range marker
  pub const END_OF_NON_EXISTENT_RANGE: u64 = 0x8C;
  /// End of Unknown Range marker
  pub const END_OF_UNKNOWN_RANGE: u64 = 0x10C;

  /// Check if this represents an end-of-range marker
  pub fn is_end_of_range(&self) -> bool {
    self.0 == Self::END_OF_NON_EXISTENT_RANGE || self.0 == Self::END_OF_UNKNOWN_RANGE
  }

  /// Extract SubgroupIdMode from bits 0-1
  pub fn subgroup_mode(&self) -> u8 {
    (self.0 & 0x03) as u8
  }

  /// Check if Object ID field is explicit (bit 2 set)
  pub fn has_explicit_object_id(&self) -> bool {
    self.0 & 0x04 != 0
  }

  /// Check if Group ID field is explicit (bit 3 set)
  pub fn has_explicit_group_id(&self) -> bool {
    self.0 & 0x08 != 0
  }

  /// Check if Publisher Priority field is explicit (bit 4 set)
  pub fn has_explicit_priority(&self) -> bool {
    self.0 & 0x10 != 0
  }

  /// Check if Extensions field is present (bit 5 set)
  pub fn has_extensions(&self) -> bool {
    self.0 & 0x20 != 0
  }

  /// Check if this is a datagram-forwarded object (bit 6 set).
  /// When set, bits 0-1 are ignored and no Subgroup ID is present.
  pub fn is_datagram(&self) -> bool {
    self.0 & 0x40 != 0
  }

  /// Compute optimal serialization flags for given object properties and prior state.
  /// Performs delta encoding to omit fields when they can be inferred.
  pub fn from_properties(
    prior: Option<&FetchObjectPriorState>,
    group_id: u64,
    subgroup_id: Option<u64>,
    object_id: u64,
    publisher_priority: u8,
    has_extensions: bool,
  ) -> Self {
    let mut flags = 0u64;

    // Group ID: explicit if no prior or different
    if prior.is_none_or(|p| p.group_id != group_id) {
      flags |= 0x08;
    }

    // Object ID: explicit if no prior or not sequential
    if prior.is_none_or(|p| p.object_id + 1 != object_id) {
      flags |= 0x04;
    }

    // Publisher Priority: explicit if no prior or different
    if prior.is_none_or(|p| p.publisher_priority != publisher_priority) {
      flags |= 0x10;
    }

    // Extensions: present if non-empty
    if has_extensions {
      flags |= 0x20;
    }

    // Subgroup ID mode
    if let Some(obj_subgroup) = subgroup_id {
      let mode: i32 = if obj_subgroup == 0 {
        0x00 // zero
      } else if prior.is_some_and(|p| p.subgroup_id == subgroup_id) {
        // Mode 0x01 means "same subgroup ID as prior". This comparison works correctly:
        // if prior had subgroup_id=None (datagram), "None != Some(obj_subgroup)" falls through.
        // Mode 0x01 is therefore never selected when the prior was a datagram object.
        0x01 // same as prior
      } else if prior.is_some_and(|p| p.subgroup_id.is_some_and(|s| s + 1 == obj_subgroup)) {
        0x02 // prior + 1
      } else {
        0x03 // explicit
      };
      flags |= mode as u64;
    } else {
      // Datagram-forwarded object
      flags |= 0x40;
      // bits 0-1 should be 0 for datagram
    }

    FetchObjectSerializationFlags(flags)
  }
}

/// Prior object state on a fetch stream, used for delta encoding.
#[derive(Debug, Clone)]
pub struct FetchObjectPriorState {
  pub group_id: u64,
  pub subgroup_id: Option<u64>,
  pub object_id: u64,
  pub publisher_priority: u8,
}

impl TryFrom<u64> for FetchObjectSerializationFlags {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    // Valid: 0x00-0x7F, 0x8C, 0x10C
    if (value <= 0x7F) || value == 0x8C || value == 0x10C {
      Ok(FetchObjectSerializationFlags(value))
    } else {
      Err(ParseError::InvalidType {
        context: "FetchObjectSerializationFlags::try_from(u64)",
        details: format!("Invalid serialization flags value: {value:#x}"),
      })
    }
  }
}
