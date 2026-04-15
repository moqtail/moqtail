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
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use crate::model::parameter::message_parameter::{
  MessageParameter, deserialize_message_parameters,
};
use bytes::{BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct RequestOk {
  pub request_id: u64,
  pub parameters: Vec<MessageParameter>,
}

impl RequestOk {
  pub fn new(request_id: u64, parameters: Vec<MessageParameter>) -> Self {
    Self {
      request_id,
      parameters,
    }
  }
}

impl ControlMessageTrait for RequestOk {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::RequestOk)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;

    payload.put_vi(self.parameters.len() as u64)?;
    for param in &self.parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "RequestOk::serialize(payload_length)",
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
    let param_count = payload.get_vi()?;
    let parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::RequestOk)?;

    Ok(Box::new(RequestOk {
      request_id,
      parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::RequestOk
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::control::constant::GroupOrder;
  use bytes::Buf;

  #[test]
  fn test_roundtrip_no_params() {
    let request_ok = RequestOk::new(12345, vec![]);

    let mut buf = request_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::RequestOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = RequestOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, request_ok);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_with_params() {
    let request_ok = RequestOk::new(
      67890,
      vec![
        MessageParameter::new_expires(3600),
        MessageParameter::new_group_order(GroupOrder::Ascending),
      ],
    );

    let mut buf = request_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::RequestOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = RequestOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, request_ok);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let request_ok = RequestOk::new(112233, vec![MessageParameter::new_expires(3600)]);

    let serialized = request_ok.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::RequestOk as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    // Slice to exactly msg_length bytes (as ControlMessage::deserialize does in production).
    let mut payload = buf.copy_to_bytes(msg_length as usize);
    let deserialized = RequestOk::parse_payload(&mut payload).unwrap();
    assert_eq!(*deserialized, request_ok);
    assert!(!payload.has_remaining());
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let request_ok = RequestOk::new(112233, vec![MessageParameter::new_expires(3600)]);
    let mut buf = request_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::RequestOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = RequestOk::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
