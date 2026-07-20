// Copyright 2026 The MOQtail Authors
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
use crate::model::common::tuple::{Tuple, TupleField};
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

/// PUBLISH_BLOCKED (0xF): a publisher tells the peer it cannot send a PUBLISH to
/// start a subscription for a track under a SUBSCRIBE_TRACKS namespace because it
/// is blocked by the peer's bidirectional stream limit. Since it always answers a
/// SUBSCRIBE_TRACKS, only the namespace suffix after the prefix is carried.
#[derive(Debug, PartialEq, Clone)]
pub struct PublishBlocked {
  pub track_namespace_suffix: Tuple,
  pub track_name: TupleField,
}

impl PublishBlocked {
  pub fn new(track_namespace_suffix: Tuple, track_name: TupleField) -> Self {
    Self {
      track_namespace_suffix,
      track_name,
    }
  }
}

impl ControlMessageTrait for PublishBlocked {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::PublishBlocked)?;

    let mut payload = BytesMut::new();
    payload.extend_from_slice(&self.track_namespace_suffix.serialize()?);
    payload.put_vi(self.track_name.len())?;
    payload.extend_from_slice(self.track_name.as_bytes());

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "PublishBlocked::serialize(payload_length)",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;
    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);

    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let track_namespace_suffix = Tuple::deserialize(payload)?;
    let name_len = payload.get_vi()? as usize;
    if payload.remaining() < name_len {
      return Err(ParseError::NotEnoughBytes {
        context: "PublishBlocked::parse_payload(track_name)",
        needed: name_len,
        available: payload.remaining(),
      });
    }
    let track_name = TupleField::new(payload.copy_to_bytes(name_len));

    Ok(Box::new(PublishBlocked {
      track_namespace_suffix,
      track_name,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::PublishBlocked
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sample() -> PublishBlocked {
    PublishBlocked::new(
      Tuple::from_utf8_path("room1/audio"),
      TupleField::from_utf8("track-42"),
    )
  }

  #[test]
  fn test_roundtrip() {
    let msg = sample();
    let mut buf = msg.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishBlocked as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = PublishBlocked::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, msg);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let msg = sample();
    let serialized = msg.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishBlocked as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = PublishBlocked::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, msg);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let msg = sample();
    let mut buf = msg.serialize().unwrap();
    let _ = buf.get_vi().unwrap();
    let _ = buf.get_u16();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    assert!(PublishBlocked::parse_payload(&mut partial).is_err());
  }
}
