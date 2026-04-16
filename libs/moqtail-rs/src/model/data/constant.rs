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

/// Subgroup Header Type
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
#[derive(Debug, PartialEq, Clone, Copy, Eq)]
pub struct SubgroupHeaderType(u8);

impl SubgroupHeaderType {
  /// Extensions present in all objects (bit 0)
  pub const EXTENSIONS: u8 = 0x01;
  /// Mask for SUBGROUP_ID_MODE (bits 1-2)
  pub const SUBGROUP_ID_MODE_MASK: u8 = 0x06;
  /// This subgroup contains the final object in the group (bit 3)
  pub const END_OF_GROUP: u8 = 0x08;
  /// Required bit that must always be set (bit 4)
  pub const REQUIRED_BIT: u8 = 0x10;
  /// Publisher priority field omitted, inherited from subscription (bit 5)
  pub const DEFAULT_PRIORITY: u8 = 0x20;
  /// Mask for bits that must be zero: bits 6-7
  const INVALID_BITS_MASK: u8 = 0xC0;
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

  /// Check if extensions are present (bit 0 set)
  pub fn has_extensions(&self) -> bool {
    self.0 & Self::EXTENSIONS != 0
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

  /// Create type from property flags.
  /// subgroup_id_mode: 0=zero, 1=firstObjId, 2=explicit
  pub fn from_properties(
    has_extensions: bool,
    subgroup_id_mode: u8,
    contains_end_of_group: bool,
    has_default_priority: bool,
  ) -> Self {
    let mut v: u8 = Self::REQUIRED_BIT;
    if has_extensions {
      v |= Self::EXTENSIONS;
    }
    v |= (subgroup_id_mode & 0x03) << 1; // bits 1-2
    if contains_end_of_group {
      v |= Self::END_OF_GROUP;
    }
    if has_default_priority {
      v |= Self::DEFAULT_PRIORITY;
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

/// Draft-16 Object Datagram Type (bitmask newtype).
///
/// Type bit layout (form `0b00X0XXXX`):
/// - Bit 0 (0x01): EXTENSIONS — Extensions field present
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
  pub const EXTENSIONS: u8 = 0x01;
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

  /// Check if extensions are present (bit 0).
  pub fn has_extensions(&self) -> bool {
    self.0 & Self::EXTENSIONS != 0
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
    has_extensions: bool,
    end_of_group: bool,
    object_id_is_zero: bool,
    default_priority: bool,
    is_status: bool,
  ) -> Result<Self, ParseError> {
    let mut v: u8 = 0;
    if has_extensions {
      v |= Self::EXTENSIONS;
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
