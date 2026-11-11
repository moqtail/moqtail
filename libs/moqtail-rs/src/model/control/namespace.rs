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

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::control_message::ControlMessageTrait;
use crate::model::common::tuple::Tuple;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::control::constant::ControlMessageType;
use crate::model::error::ParseError;
use crate::model::parameter::message_parameter::{
  MessageParameter, deserialize_message_parameters, serialize_message_parameters,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Namespace {
  pub track_namespace_suffix: Tuple,
  /// The parameter block an extension (e.g. moq-cluster) appends once negotiated;
  /// `None` sends the base message, `Some(vec![])` an empty block.
  pub parameters: Option<Vec<MessageParameter>>,
}

impl Namespace {
  pub fn new(track_namespace_suffix: Tuple) -> Self {
    Namespace {
      track_namespace_suffix,
      parameters: None,
    }
  }

  pub fn with_parameters(track_namespace_suffix: Tuple, parameters: Vec<MessageParameter>) -> Self {
    Namespace {
      track_namespace_suffix,
      parameters: Some(parameters),
    }
  }
}

impl ControlMessageTrait for Namespace {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::Namespace)?;

    let mut payload = BytesMut::new();
    payload.extend_from_slice(&self.track_namespace_suffix.serialize()?);
    if let Some(parameters) = &self.parameters {
      payload.put_vi(parameters.len())?;
      payload.extend_from_slice(&serialize_message_parameters(parameters)?);
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "Namespace::serialize(payload_length)",
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
    // The payload is bounded by Length, so anything left is the extension's block.
    let parameters = if payload.has_remaining() {
      let count = payload.get_vi()?;
      Some(deserialize_message_parameters(
        payload,
        count,
        ControlMessageType::Namespace,
      )?)
    } else {
      None
    };
    Ok(Box::new(Namespace {
      track_namespace_suffix,
      parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::Namespace
  }
}

#[cfg(test)]
mod tests {
  use crate::model::common::varint::BufVarIntExt;

  use super::*;

  #[test]
  fn test_roundtrip() {
    let track_namespace_suffix = Tuple::from_utf8_path("room1/audio");
    let namespace = Namespace::new(track_namespace_suffix);
    let mut buf = namespace.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Namespace as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = Namespace::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, namespace);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let track_namespace_suffix = Tuple::from_utf8_path("room1/audio");
    let namespace = Namespace::new(track_namespace_suffix);

    let serialized = namespace.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Namespace as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining() - 3);
    // Parsing is bounded by Length, as ControlMessage::deserialize does.
    let mut payload = buf.split_to(msg_length as usize);
    let deserialized = Namespace::parse_payload(&mut payload).unwrap();
    assert_eq!(*deserialized, namespace);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_roundtrip_with_parameters() {
    for parameters in [
      vec![],
      vec![
        MessageParameter::HopPath { hops: vec![0, 1] },
        MessageParameter::RouteCost { cost: 2 },
      ],
    ] {
      let namespace = Namespace::with_parameters(Tuple::from_utf8_path("room1/audio"), parameters);
      let mut buf = namespace.serialize().unwrap();
      buf.get_vi().unwrap();
      buf.get_u16();
      let deserialized = Namespace::parse_payload(&mut buf).unwrap();
      assert_eq!(*deserialized, namespace);
      assert!(!buf.has_remaining());
    }
  }

  #[test]
  fn test_partial_message() {
    let track_namespace_suffix = Tuple::from_utf8_path("room1/audio");
    let namespace = Namespace::new(track_namespace_suffix);
    let mut buf = namespace.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Namespace as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = Namespace::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
