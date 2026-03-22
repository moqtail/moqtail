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
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use crate::model::parameter::message_parameter::{
  MessageParameter, deserialize_message_parameters,
};
use bytes::{BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct RequestUpdate {
  pub request_id: u64,
  pub existing_request_id: u64,
  pub parameters: Vec<MessageParameter>,
}

impl RequestUpdate {
  pub fn new(
    request_id: u64,
    existing_request_id: u64,
    parameters: Vec<MessageParameter>,
  ) -> Self {
    Self {
      request_id,
      existing_request_id,
      parameters,
    }
  }
}

impl ControlMessageTrait for RequestUpdate {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::RequestUpdate)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.put_vi(self.existing_request_id)?;
    payload.put_vi(self.parameters.len())?;
    for param in &self.parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "RequestUpdate::serialize(payload_length)",
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
    let existing_request_id = payload.get_vi()?;

    let param_count = payload.get_vi()?;
    let parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::RequestUpdate)?;

    Ok(Box::new(RequestUpdate {
      request_id,
      existing_request_id,
      parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::RequestUpdate
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::location::Location;
  use crate::model::control::constant::FilterType;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let request_update = RequestUpdate::new(
      120205,
      54321,
      vec![MessageParameter::new_subscription_filter(
        FilterType::AbsoluteRange,
        Some(Location {
          group: 81,
          object: 81,
        }),
        Some(25),
      )],
    );

    let mut buf = request_update.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::RequestUpdate as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = RequestUpdate::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, request_update);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let request_update = RequestUpdate::new(
      120205,
      54321,
      vec![MessageParameter::new_subscription_filter(
        FilterType::AbsoluteRange,
        Some(Location {
          group: 81,
          object: 81,
        }),
        Some(25),
      )],
    );

    let serialized = request_update.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::RequestUpdate as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = RequestUpdate::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, request_update);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let request_update = RequestUpdate::new(
      120205,
      54321,
      vec![MessageParameter::new_subscription_filter(
        FilterType::AbsoluteRange,
        Some(Location {
          group: 81,
          object: 81,
        }),
        Some(25),
      )],
    );

    let mut buf = request_update.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::RequestUpdate as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = RequestUpdate::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}