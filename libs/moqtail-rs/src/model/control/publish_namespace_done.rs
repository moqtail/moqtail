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

use bytes::{BufMut, Bytes, BytesMut};

use super::control_message::ControlMessageTrait;
use crate::model::common::tuple::Tuple;
use crate::model::common::varint::BufMutVarIntExt;
use crate::model::control::constant::ControlMessageType;
use crate::model::error::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub struct PublishNamespaceDone {
  pub track_namespace: Tuple,
}
impl PublishNamespaceDone {
  pub fn new(track_namespace: Tuple) -> Self {
    PublishNamespaceDone { track_namespace }
  }
}
impl ControlMessageTrait for PublishNamespaceDone {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::PublishNamespaceDone)?;

    let mut payload = BytesMut::new();
    payload.extend_from_slice(&self.track_namespace.serialize()?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "PublishNamespaceDone::serialize(payload_length)",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;
    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);

    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let track_namespace = Tuple::deserialize(payload)?;
    Ok(Box::new(PublishNamespaceDone { track_namespace }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::PublishNamespaceDone
  }
}

#[cfg(test)]
mod tests {
  use crate::model::common::varint::BufVarIntExt;

  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let track_namespace = Tuple::from_utf8_path("un/announce/me");
    let publish_namespace_done = PublishNamespaceDone { track_namespace };
    let mut buf = publish_namespace_done.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishNamespaceDone as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = PublishNamespaceDone::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, publish_namespace_done);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let track_namespace = Tuple::from_utf8_path("un/announce/me");
    let publish_namespace_done = PublishNamespaceDone { track_namespace };

    let serialized = publish_namespace_done.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishNamespaceDone as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = PublishNamespaceDone::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, publish_namespace_done);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let track_namespace = Tuple::from_utf8_path("un/announce/me");
    let publish_namespace_done = PublishNamespaceDone { track_namespace };
    let mut buf = publish_namespace_done.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishNamespaceDone as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = PublishNamespaceDone::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
