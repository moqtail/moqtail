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

use std::convert::TryFrom;

use crate::model::error::ParseError;

#[repr(u64)]
#[derive(Clone, Debug, Copy, PartialEq)]
pub enum LOCPropertyId {
  Timestamp = 0x06,
  Timescale = 0x08,
  VideoFrameMarking = 0x0A,
  AudioLevel = 0x0C,
  VideoConfig = 0x0D,
}

impl TryFrom<u64> for LOCPropertyId {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x06 => Ok(LOCPropertyId::Timestamp),
      0x08 => Ok(LOCPropertyId::Timescale),
      0x0A => Ok(LOCPropertyId::VideoFrameMarking),
      0x0C => Ok(LOCPropertyId::AudioLevel),
      0x0D => Ok(LOCPropertyId::VideoConfig),
      _ => Err(ParseError::InvalidType {
        context: "LOCPropertyId::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<LOCPropertyId> for u64 {
  fn from(value: LOCPropertyId) -> Self {
    value as u64
  }
}

/// MOQT property type values.
/// Even type values use VarInt KVP encoding; odd type values use Bytes KVP encoding.
#[repr(u64)]
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum TrackPropertyType {
  ObjectDeliveryTimeout = 0x02, // Track scope, VarInt (ms, 0 = no timeout)
  MaxCacheDuration = 0x04,      // Track scope, VarInt (ms)
  SubgroupDeliveryTimeout = 0x06, // Track scope, VarInt (ms, 0 = no timeout)
  ImmutableProperties = 0x0B,   // Track+Object scope, Bytes (nested KVPs)
  DefaultPublisherPriority = 0x0E, // Track scope, VarInt (0-255)
  DefaultPublisherGroupOrder = 0x22, // Track scope, VarInt (1=Ascending, 2=Descending)
  DynamicGroups = 0x30,         // Track scope, VarInt (0 or 1)
  PriorGroupIdGap = 0x3C,       // Object scope, VarInt
  PriorObjectIdGap = 0x3E,      // Object scope, VarInt
}

impl TryFrom<u64> for TrackPropertyType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x02 => Ok(TrackPropertyType::ObjectDeliveryTimeout),
      0x04 => Ok(TrackPropertyType::MaxCacheDuration),
      0x06 => Ok(TrackPropertyType::SubgroupDeliveryTimeout),
      0x0B => Ok(TrackPropertyType::ImmutableProperties),
      0x0E => Ok(TrackPropertyType::DefaultPublisherPriority),
      0x22 => Ok(TrackPropertyType::DefaultPublisherGroupOrder),
      0x30 => Ok(TrackPropertyType::DynamicGroups),
      0x3C => Ok(TrackPropertyType::PriorGroupIdGap),
      0x3E => Ok(TrackPropertyType::PriorObjectIdGap),
      _ => Err(ParseError::InvalidType {
        context: "TrackPropertyType::try_from(u64)",
        details: format!("Unknown property type, got {value}"),
      }),
    }
  }
}

impl From<TrackPropertyType> for u64 {
  fn from(value: TrackPropertyType) -> Self {
    value as u64
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_track_property_type_roundtrip() {
    let known = [
      (0x02u64, TrackPropertyType::ObjectDeliveryTimeout),
      (0x04, TrackPropertyType::MaxCacheDuration),
      (0x06, TrackPropertyType::SubgroupDeliveryTimeout),
      (0x0B, TrackPropertyType::ImmutableProperties),
      (0x0E, TrackPropertyType::DefaultPublisherPriority),
      (0x22, TrackPropertyType::DefaultPublisherGroupOrder),
      (0x30, TrackPropertyType::DynamicGroups),
      (0x3C, TrackPropertyType::PriorGroupIdGap),
      (0x3E, TrackPropertyType::PriorObjectIdGap),
    ];
    for (raw, expected) in known {
      let parsed = TrackPropertyType::try_from(raw).unwrap();
      assert_eq!(parsed, expected);
      assert_eq!(u64::from(parsed), raw);
    }
  }

  #[test]
  fn test_track_property_type_unknown() {
    assert!(TrackPropertyType::try_from(0xFF).is_err());
    assert!(TrackPropertyType::try_from(0x00).is_err());
  }
}
