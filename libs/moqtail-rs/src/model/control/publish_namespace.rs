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

use super::control_message::ControlMessageTrait;
use crate::model::common::pair::KeyValuePair;
use crate::model::common::tuple::Tuple;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::control::constant::ControlMessageType;
use crate::model::error::ParseError;
use bytes::{BufMut, Bytes, BytesMut};

#[derive(Debug, Clone, PartialEq)]
pub struct PublishNamespace {
  pub request_id: u64,
  pub track_namespace: Tuple,
  pub parameters: Vec<KeyValuePair>,
}

impl PublishNamespace {
  /// * Creates a new PublishNamespace message.
  pub fn new(request_id: u64, track_namespace: Tuple, parameters: &[KeyValuePair]) -> Self {
    PublishNamespace {
      request_id,
      track_namespace,
      parameters: parameters.to_vec(),
    }
  }
}

impl ControlMessageTrait for PublishNamespace {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::PublishNamespace)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.extend_from_slice(&self.track_namespace.serialize()?);
    payload.put_vi(self.parameters.len())?;

    for param in &self.parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "PublishNamespace::serialize(payload_length)",
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
    let track_namespace = Tuple::deserialize(payload)?;

    let param_count_u64 = payload.get_vi()?;
    let param_count: usize =
      param_count_u64
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "PublishNamespace::deserialize(param_count)",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;

    let mut parameters = Vec::with_capacity(param_count);
    for _ in 0..param_count {
      let param = KeyValuePair::deserialize(payload)?;
      parameters.push(param);
    }

    Ok(Box::new(PublishNamespace {
      request_id,
      track_namespace,
      parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::PublishNamespace
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let request_id = 12345;
    let track_namespace = Tuple::from_utf8_path("god/dayyum");
    let parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ];
    let announce = PublishNamespace {
      request_id,
      track_namespace,
      parameters,
    };

    let mut buf = announce.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishNamespace as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = PublishNamespace::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, announce);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let request_id = 12345;
    let track_namespace = Tuple::from_utf8_path("god/dayyum");
    let parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ];
    let announce = PublishNamespace {
      request_id,
      track_namespace,
      parameters,
    };

    let serialized = announce.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishNamespace as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = PublishNamespace::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, announce);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let request_id = 12345;
    let track_namespace = Tuple::from_utf8_path("god/dayyum");
    let parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ];
    let announce = PublishNamespace {
      request_id,
      track_namespace,
      parameters,
    };

    let mut buf = announce.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishNamespace as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = PublishNamespace::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
