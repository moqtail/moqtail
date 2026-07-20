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

use super::constant::ControlMessageType;
use super::control_message::ControlMessageTrait;
use crate::model::common::tuple::Tuple;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use crate::model::parameter::message_parameter::{
  MessageParameter, deserialize_message_parameters, serialize_message_parameters,
};
use bytes::{BufMut, Bytes, BytesMut};

/// SUBSCRIBE_TRACKS (0x51) requests a PUBLISH for every track under the matching
/// namespace prefix, present and future. It shares SUBSCRIBE_NAMESPACE's wire
/// shape but has an independent prefix-overlap space.
#[derive(Debug, PartialEq, Clone)]
pub struct SubscribeTracks {
  pub request_id: u64,
  pub track_namespace_prefix: Tuple,
  pub parameters: Vec<MessageParameter>,
}

impl SubscribeTracks {
  pub fn new(
    request_id: u64,
    track_namespace_prefix: Tuple,
    parameters: Vec<MessageParameter>,
  ) -> Self {
    Self {
      request_id,
      track_namespace_prefix,
      parameters,
    }
  }
}

impl ControlMessageTrait for SubscribeTracks {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::SubscribeTracks)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.extend_from_slice(&self.track_namespace_prefix.serialize()?);

    payload.put_vi(self.parameters.len())?;
    payload.extend_from_slice(&serialize_message_parameters(&self.parameters)?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "SubscribeTracks::serialize",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;
    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);
    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let request_id = payload.get_vi()?;
    let track_namespace_prefix = Tuple::deserialize(payload)?;
    if track_namespace_prefix.fields.len() > 32 {
      return Err(ParseError::ProtocolViolation {
        context: "SubscribeTracks::parse_payload",
        details: format!(
          "Track namespace prefix has {} fields, maximum is 32",
          track_namespace_prefix.fields.len()
        ),
      });
    }
    let param_count = payload.get_vi()?;
    let parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::SubscribeTracks)?;

    Ok(Box::new(SubscribeTracks {
      request_id,
      track_namespace_prefix,
      parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::SubscribeTracks
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  fn sample() -> SubscribeTracks {
    SubscribeTracks::new(
      241421,
      Tuple::from_utf8_path("pre/fix/me"),
      vec![MessageParameter::new_forward(true)],
    )
  }

  #[test]
  fn test_roundtrip() {
    let subscribe_tracks = sample();
    let mut buf = subscribe_tracks.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeTracks as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = SubscribeTracks::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe_tracks);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let subscribe_tracks = sample();
    let serialized = subscribe_tracks.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeTracks as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = SubscribeTracks::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe_tracks);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let subscribe_tracks = sample();
    let mut buf = subscribe_tracks.serialize().unwrap();
    let _ = buf.get_vi().unwrap();
    let _ = buf.get_u16();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    assert!(SubscribeTracks::parse_payload(&mut partial).is_err());
  }
}
