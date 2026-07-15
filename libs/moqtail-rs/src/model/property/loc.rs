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
  CaptureTimestamp { value: u64 },
  VideoFrameMarking { value: u64 },
  AudioLevel { value: u64 },
  VideoConfig { value: Bytes },
}

impl LOCProperty {
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    match self {
      LOCProperty::CaptureTimestamp { value } => {
        buf.put_vi(LOCPropertyId::CaptureTimestamp)?;
        buf.put_vi(*value)?;
      }
      LOCProperty::VideoFrameMarking { value } => {
        buf.put_vi(LOCPropertyId::VideoFrameMarking)?;
        buf.put_vi(*value)?;
      }
      LOCProperty::AudioLevel { value } => {
        buf.put_vi(LOCPropertyId::AudioLevel)?;
        buf.put_vi(*value)?;
      }
      LOCProperty::VideoConfig { value } => {
        buf.put_vi(LOCPropertyId::VideoConfig)?;
        buf.put_vi(value.len())?;
        buf.extend_from_slice(value);
      }
    }
    Ok(buf.freeze())
  }
  pub fn deserialize(payload: &mut Bytes) -> Result<Self, ParseError> {
    let id = payload.get_vi()?;
    let loc_property_id = LOCPropertyId::try_from(id)?;

    match loc_property_id {
      LOCPropertyId::VideoConfig => {
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
        let value = payload.copy_to_bytes(payload_length);
        Ok(LOCProperty::VideoConfig { value })
      }
      LOCPropertyId::AudioLevel => {
        let value = payload.get_vi()?;
        Ok(LOCProperty::AudioLevel { value })
      }
      LOCPropertyId::CaptureTimestamp => {
        let value = payload.get_vi()?;
        Ok(LOCProperty::CaptureTimestamp { value })
      }
      LOCPropertyId::VideoFrameMarking => {
        let value = payload.get_vi()?;
        Ok(LOCProperty::VideoFrameMarking { value })
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
          LOCPropertyId::CaptureTimestamp => Ok(LOCProperty::CaptureTimestamp { value }),
          LOCPropertyId::VideoFrameMarking => Ok(LOCProperty::VideoFrameMarking { value }),
          _ => Err(ParseError::InvalidType {
            context: "LOCProperty::try_from(KeyValuePair::VarInt)",
            details: format!("Invalid type, got {type_value}"),
          }),
        }
      }
      KeyValuePair::Bytes { type_value, value } => {
        let id = LOCPropertyId::try_from(type_value)?;
        match id {
          LOCPropertyId::VideoConfig => Ok(LOCProperty::VideoConfig { value }),
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
