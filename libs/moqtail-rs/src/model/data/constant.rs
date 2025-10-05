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
