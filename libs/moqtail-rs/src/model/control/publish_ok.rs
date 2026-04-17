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
pub struct PublishOk {
  pub request_id: u64,
  pub parameters: Vec<MessageParameter>,
}

impl PublishOk {
  pub fn new(request_id: u64, parameters: Vec<MessageParameter>) -> Self {
    Self {
      request_id,
      parameters,
    }
  }
}

impl ControlMessageTrait for PublishOk {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::PublishOk)?;

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
        context: "PublishOk::serialize(payload_length)",
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
      deserialize_message_parameters(payload, param_count, ControlMessageType::PublishOk)?;

    Ok(Box::new(PublishOk {
      request_id,
      parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::PublishOk
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::location::Location;
  use crate::model::control::constant::{FilterType, GroupOrder};
  use bytes::Buf;

  #[test]
  fn test_roundtrip_no_params() {
    let publish_ok = PublishOk::new(123, vec![]);

    let mut buf = publish_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = PublishOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, publish_ok);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_with_params() {
    let publish_ok = PublishOk::new(
      456,
      vec![
        MessageParameter::new_forward(true),
        MessageParameter::new_subscriber_priority(5),
        MessageParameter::new_group_order(GroupOrder::Ascending),
        MessageParameter::new_subscription_filter(FilterType::LatestObject, None, None),
      ],
    );

    let mut buf = publish_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = PublishOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, publish_ok);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_absolute_range_filter() {
    let publish_ok = PublishOk::new(
      789,
      vec![
        MessageParameter::new_subscription_filter(
          FilterType::AbsoluteRange,
          Some(Location::new(5, 10)),
          Some(100),
        ),
        MessageParameter::new_delivery_timeout(5000),
      ],
    );

    let mut buf = publish_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = PublishOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, publish_ok);
    assert!(!buf.has_remaining());
  }
}
