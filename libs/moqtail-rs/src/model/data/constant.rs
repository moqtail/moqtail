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

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SubgroupHeaderType {
  /// No Subgroup ID field (Subgroup ID = 0), No Extensions, No End of Group
  Type0x10 = 0x10,
  /// No Subgroup ID field (Subgroup ID = 0), Extensions Present, No End of Group
  Type0x11 = 0x11,
  /// No Subgroup ID field (Subgroup ID = First Object ID), No Extensions, No End of Group
  Type0x12 = 0x12,
  /// No Subgroup ID field (Subgroup ID = First Object ID), Extensions Present, No End of Group
  Type0x13 = 0x13,
  /// Explicit Subgroup ID field, No Extensions, No End of Group
  Type0x14 = 0x14,
  /// Explicit Subgroup ID field, Extensions Present, No End of Group
  Type0x15 = 0x15,
  /// No Subgroup ID field (Subgroup ID = 0), No Extensions, Contains End of Group
  Type0x18 = 0x18,
  /// No Subgroup ID field (Subgroup ID = 0), Extensions Present, Contains End of Group
  Type0x19 = 0x19,
  /// No Subgroup ID field (Subgroup ID = First Object ID), No Extensions, Contains End of Group
  Type0x1A = 0x1A,
  /// No Subgroup ID field (Subgroup ID = First Object ID), Extensions Present, Contains End of Group
  Type0x1B = 0x1B,
  /// Explicit Subgroup ID field, No Extensions, Contains End of Group
  Type0x1C = 0x1C,
  /// Explicit Subgroup ID field, Extensions Present, Contains End of Group
  Type0x1D = 0x1D,
}

impl SubgroupHeaderType {
  pub fn has_explicit_subgroup_id(&self) -> bool {
    matches!(
      self,
      Self::Type0x14 | Self::Type0x15 | Self::Type0x1C | Self::Type0x1D
    )
  }

  pub fn has_extensions(&self) -> bool {
    matches!(
      self,
      Self::Type0x11
        | Self::Type0x13
        | Self::Type0x15
        | Self::Type0x19
        | Self::Type0x1B
        | Self::Type0x1D
    )
  }

  pub fn contains_end_of_group(&self) -> bool {
    matches!(
      self,
      Self::Type0x18
        | Self::Type0x19
        | Self::Type0x1A
        | Self::Type0x1B
        | Self::Type0x1C
        | Self::Type0x1D
    )
  }

  pub fn subgroup_id_is_zero(&self) -> bool {
    matches!(
      self,
      Self::Type0x10 | Self::Type0x11 | Self::Type0x18 | Self::Type0x19
    )
  }

  pub fn subgroup_id_is_first_object_id(&self) -> bool {
    matches!(
      self,
      Self::Type0x12 | Self::Type0x13 | Self::Type0x1A | Self::Type0x1B
    )
  }
}

impl TryFrom<u64> for SubgroupHeaderType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
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
      _ => Err(ParseError::InvalidType {
        context: "SubgroupHeaderType::try_from(u64)",
        details: format!("Invalid type, got {value}"),
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

/// Draft-16 Object Datagram Type
///
/// Type bit layout (form 0b00X0XXXX):
/// - Bit 0 (0x01): EXTENSIONS - Extensions field present
/// - Bit 1 (0x02): END_OF_GROUP - Last object in group
/// - Bit 2 (0x04): ZERO_OBJECT_ID - Object ID omitted (assumed 0)
/// - Bit 3 (0x08): DEFAULT_PRIORITY - Publisher Priority omitted (inherited)
/// - Bit 5 (0x20): STATUS - Object Status replaces Object Payload
///
/// Invalid combinations:
/// - STATUS (0x20) + END_OF_GROUP (0x02) together is a PROTOCOL_VIOLATION
/// - Types outside the form 0b00X0XXXX are invalid
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ObjectDatagramType {
  Type0x00 = 0x00,
  Type0x01 = 0x01,
  Type0x02 = 0x02,
  Type0x03 = 0x03,
  Type0x04 = 0x04,
  Type0x05 = 0x05,
  Type0x06 = 0x06,
  Type0x07 = 0x07,
  Type0x08 = 0x08,
  Type0x09 = 0x09,
  Type0x0A = 0x0A,
  Type0x0B = 0x0B,
  Type0x0C = 0x0C,
  Type0x0D = 0x0D,
  Type0x0E = 0x0E,
  Type0x0F = 0x0F,
  Type0x20 = 0x20,
  Type0x21 = 0x21,
  Type0x24 = 0x24,
  Type0x25 = 0x25,
  Type0x28 = 0x28,
  Type0x29 = 0x29,
  Type0x2C = 0x2C,
  Type0x2D = 0x2D,
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

  /// Check if Object ID is absent (bit 2 set).
  /// When true, Object ID is omitted and assumed to be 0.
  pub fn is_zero_object_id(&self) -> bool {
    (*self as u64) & 0x04 != 0
  }

  /// Check if Publisher Priority is omitted (bit 3 set).
  /// When true, the priority is inherited from the control message.
  pub fn has_default_priority(&self) -> bool {
    (*self as u64) & 0x08 != 0
  }

  /// Check if the datagram carries Object Status instead of payload (bit 5 set).
  pub fn is_status(&self) -> bool {
    (*self as u64) & 0x20 != 0
  }

  /// Create type from properties.
  /// Returns error if STATUS and END_OF_GROUP are both true (PROTOCOL_VIOLATION).
  pub fn from_properties(
    has_extensions: bool,
    end_of_group: bool,
    object_id_is_zero: bool,
    default_priority: bool,
    is_status: bool,
  ) -> Result<Self, ParseError> {
    if is_status && end_of_group {
      return Err(ParseError::ProtocolViolation {
        context: "ObjectDatagramType::from_properties",
        details: "STATUS and END_OF_GROUP cannot both be set".to_string(),
      });
    }
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
    if default_priority {
      type_val |= 0x08;
    }
    if is_status {
      type_val |= 0x20;
    }
    Self::try_from(type_val)
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
      0x08 => Ok(ObjectDatagramType::Type0x08),
      0x09 => Ok(ObjectDatagramType::Type0x09),
      0x0A => Ok(ObjectDatagramType::Type0x0A),
      0x0B => Ok(ObjectDatagramType::Type0x0B),
      0x0C => Ok(ObjectDatagramType::Type0x0C),
      0x0D => Ok(ObjectDatagramType::Type0x0D),
      0x0E => Ok(ObjectDatagramType::Type0x0E),
      0x0F => Ok(ObjectDatagramType::Type0x0F),
      0x20 => Ok(ObjectDatagramType::Type0x20),
      0x21 => Ok(ObjectDatagramType::Type0x21),
      0x24 => Ok(ObjectDatagramType::Type0x24),
      0x25 => Ok(ObjectDatagramType::Type0x25),
      0x28 => Ok(ObjectDatagramType::Type0x28),
      0x29 => Ok(ObjectDatagramType::Type0x29),
      0x2C => Ok(ObjectDatagramType::Type0x2C),
      0x2D => Ok(ObjectDatagramType::Type0x2D),
      _ => Err(ParseError::InvalidType {
        context: "ObjectDatagramType::try_from(u64)",
        details: format!("Invalid type, got {value:#x}"),
      }),
    }
  }
}

impl From<ObjectDatagramType> for u64 {
  fn from(dtype: ObjectDatagramType) -> Self {
    dtype as u64
  }
}
