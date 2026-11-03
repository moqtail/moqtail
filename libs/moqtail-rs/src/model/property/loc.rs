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

use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, Bytes, BytesMut};

use super::constant::LOCPropertyId;

#[derive(Clone, Debug, PartialEq)]
pub enum LOCProperty {
  Timestamp {
    value: u64,
  },
  Timescale {
    value: u64,
  },
  /// The RFC 9626 frame-marking octets (1 to 4).
  VideoFrameMarking {
    value: Bytes,
  },
  AudioLevel {
    value: u64,
  },
  VideoConfig {
    value: Bytes,
  },
  AudioConfig {
    value: Bytes,
  },
}

impl LOCProperty {
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    match self {
      LOCProperty::Timestamp { value } => {
        buf.put_vi(LOCPropertyId::Timestamp)?;
        buf.put_vi(*value)?;
      }
      LOCProperty::Timescale { value } => {
        buf.put_vi(LOCPropertyId::Timescale)?;
        buf.put_vi(*value)?;
      }
      LOCProperty::AudioLevel { value } => {
        buf.put_vi(LOCPropertyId::AudioLevel)?;
        buf.put_vi(*value)?;
      }
      LOCProperty::VideoFrameMarking { value }
      | LOCProperty::VideoConfig { value }
      | LOCProperty::AudioConfig { value } => {
        buf.put_vi(self.id())?;
        buf.put_vi(value.len())?;
        buf.extend_from_slice(value);
      }
    }
    Ok(buf.freeze())
  }
  fn id(&self) -> LOCPropertyId {
    match self {
      LOCProperty::Timestamp { .. } => LOCPropertyId::Timestamp,
      LOCProperty::Timescale { .. } => LOCPropertyId::Timescale,
      LOCProperty::VideoFrameMarking { .. } => LOCPropertyId::VideoFrameMarking,
      LOCProperty::AudioLevel { .. } => LOCPropertyId::AudioLevel,
      LOCProperty::VideoConfig { .. } => LOCPropertyId::VideoConfig,
      LOCProperty::AudioConfig { .. } => LOCPropertyId::AudioConfig,
    }
  }

  /// True if the frame decodes without temporally prior frames (the RFC 9626 I bit).
  pub fn is_independent_frame(&self) -> bool {
    matches!(self, LOCProperty::VideoFrameMarking { value } if value.first().is_some_and(|b| b & 0x20 != 0))
  }

  pub fn deserialize(payload: &mut Bytes) -> Result<Self, ParseError> {
    let id = payload.get_vi()?;
    let loc_property_id = LOCPropertyId::try_from(id)?;

    match loc_property_id {
      LOCPropertyId::VideoFrameMarking
      | LOCPropertyId::VideoConfig
      | LOCPropertyId::AudioConfig => {
        let payload_length = payload.get_vi()?;
        let payload_length: usize =
          payload_length
            .try_into()
            .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
              context: "LOCProperty::deserialize(payload_length)",
              from_type: "u64",
              to_type: "usize",
              details: e.to_string(),
            })?;
        if payload.remaining() < payload_length {
          return Err(ParseError::NotEnoughBytes {
            context: "LOCProperty::deserialize(value)",
            needed: payload_length,
            available: payload.remaining(),
          });
        }
        let value = payload.copy_to_bytes(payload_length);
        Ok(match loc_property_id {
          LOCPropertyId::VideoFrameMarking => LOCProperty::VideoFrameMarking { value },
          LOCPropertyId::AudioConfig => LOCProperty::AudioConfig { value },
          _ => LOCProperty::VideoConfig { value },
        })
      }
      LOCPropertyId::AudioLevel => {
        let value = payload.get_vi()?;
        Ok(LOCProperty::AudioLevel { value })
      }
      LOCPropertyId::Timestamp => {
        let value = payload.get_vi()?;
        Ok(LOCProperty::Timestamp { value })
      }
      LOCPropertyId::Timescale => {
        let value = payload.get_vi()?;
        Ok(LOCProperty::Timescale { value })
      }
    }
  }
}

impl TryFrom<KeyValuePair> for LOCProperty {
  type Error = ParseError;
  fn try_from(pair: KeyValuePair) -> Result<Self, Self::Error> {
    match pair {
      KeyValuePair::VarInt { type_value, value } => {
        let id = LOCPropertyId::try_from(type_value)?;
        match id {
          LOCPropertyId::AudioLevel => Ok(LOCProperty::AudioLevel { value }),
          LOCPropertyId::Timestamp => Ok(LOCProperty::Timestamp { value }),
          LOCPropertyId::Timescale => Ok(LOCProperty::Timescale { value }),
          _ => Err(ParseError::InvalidType {
            context: "LOCProperty::try_from(KeyValuePair::VarInt)",
            details: format!("Invalid type, got {type_value}"),
          }),
        }
      }
      KeyValuePair::Bytes { type_value, value } => {
        let id = LOCPropertyId::try_from(type_value)?;
        match id {
          LOCPropertyId::VideoFrameMarking => Ok(LOCProperty::VideoFrameMarking { value }),
          LOCPropertyId::VideoConfig => Ok(LOCProperty::VideoConfig { value }),
          LOCPropertyId::AudioConfig => Ok(LOCProperty::AudioConfig { value }),
          _ => Err(ParseError::InvalidType {
            context: "LOCProperty::try_from(KeyValuePair::Bytes)",
            details: format!("Invalid type, got {type_value}"),
          }),
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let value = 144;
    let loc = LOCProperty::AudioLevel { value };

    let mut buf = loc.serialize().unwrap();
    let deserialized = LOCProperty::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, loc);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let value = 144;
    let loc = LOCProperty::AudioLevel { value };

    let serialized = loc.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let deserialized = LOCProperty::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, loc);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_bytes_properties_roundtrip() {
    for loc in [
      LOCProperty::VideoFrameMarking {
        value: Bytes::from_static(&[0xE0]),
      },
      LOCProperty::VideoConfig {
        value: Bytes::from_static(&[1, 2, 3]),
      },
      LOCProperty::AudioConfig {
        value: Bytes::from_static(&[0x11, 0x90]),
      },
    ] {
      let mut buf = loc.serialize().unwrap();
      assert_eq!(LOCProperty::deserialize(&mut buf).unwrap(), loc);
      assert!(!buf.has_remaining());
    }
  }

  #[test]
  fn test_frame_marking_wire_and_independent_bit() {
    let key = LOCProperty::VideoFrameMarking {
      value: Bytes::from_static(&[0xE0]),
    };
    assert_eq!(key.serialize().unwrap().as_ref(), &[0x09, 0x01, 0xE0]);
    assert!(key.is_independent_frame());
    let delta = LOCProperty::VideoFrameMarking {
      value: Bytes::from_static(&[0xC0]),
    };
    assert!(!delta.is_independent_frame());
    assert!(
      !LOCProperty::VideoFrameMarking {
        value: Bytes::new()
      }
      .is_independent_frame()
    );
    assert!(!LOCProperty::AudioLevel { value: 0x20 }.is_independent_frame());
  }

  #[test]
  fn test_truncated_bytes_value_errors() {
    let mut buf = Bytes::from_static(&[0x0D, 0x05, 1, 2]);
    assert!(LOCProperty::deserialize(&mut buf).is_err());
  }

  #[test]
  fn test_partial_message() {
    let value = 144;
    let loc = LOCProperty::AudioLevel { value };
    let buf = loc.serialize().unwrap();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = LOCProperty::deserialize(&mut partial);
    assert!(deserialized.is_err());
  }
}
