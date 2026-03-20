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
pub enum LOCHeaderExtensionId {
  CaptureTimestamp = 2,  // Section 2.3.1.1 - Common Header
  VideoFrameMarking = 4, // Section 2.3.2.2 - Video Header
  AudioLevel = 6,        // Section 2.3.3.1 - Audio Header
  VideoConfig = 13,      // Section 2.3.2.1 - Video Header
}

impl TryFrom<u64> for LOCHeaderExtensionId {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      2 => Ok(LOCHeaderExtensionId::CaptureTimestamp),
      4 => Ok(LOCHeaderExtensionId::VideoFrameMarking),
      6 => Ok(LOCHeaderExtensionId::AudioLevel),
      13 => Ok(LOCHeaderExtensionId::VideoConfig),
      _ => Err(ParseError::InvalidType {
        context: "LOCHeaderExtensionId::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<LOCHeaderExtensionId> for u64 {
  fn from(value: LOCHeaderExtensionId) -> Self {
    value as u64
  }
}

/// MOQT extension header type values.
/// Even type values use VarInt KVP encoding; odd type values use Bytes KVP encoding.
#[repr(u64)]
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum TrackExtensionType {
  DeliveryTimeout = 0x02,            // Track scope, VarInt (ms, must be > 0)
  MaxCacheDuration = 0x04,           // Track scope, VarInt (ms)
  ImmutableExtensions = 0x0B,        // Track+Object scope, Bytes (nested KVPs)
  DefaultPublisherPriority = 0x0E,   // Track scope, VarInt (0-255)
  DefaultPublisherGroupOrder = 0x22, // Track scope, VarInt (1=Ascending, 2=Descending)
  DynamicGroups = 0x30,              // Track scope, VarInt (0 or 1)
  PriorGroupIdGap = 0x3C,            // Object scope, VarInt
  PriorObjectIdGap = 0x3E,           // Object scope, VarInt
}

impl TryFrom<u64> for TrackExtensionType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x02 => Ok(TrackExtensionType::DeliveryTimeout),
      0x04 => Ok(TrackExtensionType::MaxCacheDuration),
      0x0B => Ok(TrackExtensionType::ImmutableExtensions),
      0x0E => Ok(TrackExtensionType::DefaultPublisherPriority),
      0x22 => Ok(TrackExtensionType::DefaultPublisherGroupOrder),
      0x30 => Ok(TrackExtensionType::DynamicGroups),
      0x3C => Ok(TrackExtensionType::PriorGroupIdGap),
      0x3E => Ok(TrackExtensionType::PriorObjectIdGap),
      _ => Err(ParseError::InvalidType {
        context: "TrackExtensionType::try_from(u64)",
        details: format!("Unknown extension type, got {value}"),
      }),
    }
  }
}

impl From<TrackExtensionType> for u64 {
  fn from(value: TrackExtensionType) -> Self {
    value as u64
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_track_extension_type_roundtrip() {
    let known = [
      (0x02u64, TrackExtensionType::DeliveryTimeout),
      (0x04, TrackExtensionType::MaxCacheDuration),
      (0x0B, TrackExtensionType::ImmutableExtensions),
      (0x0E, TrackExtensionType::DefaultPublisherPriority),
      (0x22, TrackExtensionType::DefaultPublisherGroupOrder),
      (0x30, TrackExtensionType::DynamicGroups),
      (0x3C, TrackExtensionType::PriorGroupIdGap),
      (0x3E, TrackExtensionType::PriorObjectIdGap),
    ];
    for (raw, expected) in known {
      let parsed = TrackExtensionType::try_from(raw).unwrap();
      assert_eq!(parsed, expected);
      assert_eq!(u64::from(parsed), raw);
    }
  }

  #[test]
  fn test_track_extension_type_unknown() {
    assert!(TrackExtensionType::try_from(0xFF).is_err());
    assert!(TrackExtensionType::try_from(0x00).is_err());
  }
}
