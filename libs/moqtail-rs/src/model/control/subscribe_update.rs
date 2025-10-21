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
use crate::model::common::location::Location;
use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct SubscribeUpdate {
  pub request_id: u64,
  pub subscription_request_id: u64,
  pub start_location: Location,
  pub end_group: u64,
  pub subscriber_priority: u8,
  pub forward: bool,
  pub subscribe_parameters: Vec<KeyValuePair>,
}

impl SubscribeUpdate {
  pub fn new(
    request_id: u64,
    subscription_request_id: u64,
    start_location: Location,
    end_group: u64,
    subscriber_priority: u8,
    forward: bool,
    subscribe_parameters: Vec<KeyValuePair>,
  ) -> Self {
    Self {
      request_id,
      subscription_request_id,
      start_location,
      end_group,
      subscriber_priority,
      forward,
      subscribe_parameters,
    }
  }
}

impl ControlMessageTrait for SubscribeUpdate {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::SubscribeUpdate)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.put_vi(self.subscription_request_id)?;
    payload.extend_from_slice(&self.start_location.serialize()?);
    payload.put_vi(self.end_group)?;
    payload.put_u8(self.subscriber_priority);
    payload.put_u8(if self.forward { 1u8 } else { 0u8 });
    payload.put_vi(self.subscribe_parameters.len())?;
    for param in &self.subscribe_parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "SubscribeUpdate::serialize(payload_length)",
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
    let subscription_request_id = payload.get_vi()?;
    let start_location = Location::deserialize(payload)?;
    let end_group = payload.get_vi()?;

    if payload.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "SubscribeUpdate::parse_payload(subscriber_priority)",
        needed: 1,
        available: 0,
      });
    }
    let subscriber_priority = payload.get_u8();

    if payload.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "SubscribeUpdate::parse_payload(forward)",
        needed: 1,
        available: 0,
      });
    }

    let forward_raw = payload.get_u8();
    let forward = match forward_raw {
      0 => false,
      1 => true,
      _ => {
        return Err(ParseError::ProtocolViolation {
          context: "Subscribe::parse_payload(forward)",
          details: format!("Invalid value: {forward_raw}"),
        });
      }
    };

    let param_count_u64 = payload.get_vi()?;
    let param_count: usize =
      param_count_u64
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "SubscribeUpdate::parse_payload(param_count)",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;

    let mut subscribe_parameters = Vec::with_capacity(param_count);
    for _ in 0..param_count {
      let param = KeyValuePair::deserialize(payload)?;
      subscribe_parameters.push(param);
    }

    Ok(Box::new(SubscribeUpdate {
      request_id,
      subscription_request_id,
      start_location,
      end_group,
      subscriber_priority,
      forward,
      subscribe_parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::SubscribeUpdate
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let request_id = 120205;
    let subscription_request_id = 54321;
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let end_group = 25;
    let subscriber_priority = 31;
    let forward = true;
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"I'll sync you up")).unwrap(),
    ];
    let subscribe_update = SubscribeUpdate {
      request_id,
      subscription_request_id,
      start_location,
      end_group,
      subscriber_priority,
      forward,
      subscribe_parameters,
    };

    let mut buf = subscribe_update.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeUpdate as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = SubscribeUpdate::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe_update);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let request_id = 120205;
    let subscription_request_id = 54321;
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let end_group = 25;
    let subscriber_priority = 31;
    let forward = true;
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"I'll sync you up")).unwrap(),
    ];
    let subscribe_update = SubscribeUpdate {
      request_id,
      subscription_request_id,
      start_location,
      end_group,
      subscriber_priority,
      forward,
      subscribe_parameters,
    };

    let serialized = subscribe_update.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeUpdate as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = SubscribeUpdate::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe_update);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let request_id = 120205;
    let subscription_request_id = 54321;
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let end_group = 25;
    let subscriber_priority = 31;
    let forward = true;
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"I'll sync you up")).unwrap(),
    ];
    let subscribe_update = SubscribeUpdate {
      request_id,
      subscription_request_id,
      start_location,
      end_group,
      subscriber_priority,
      forward,
      subscribe_parameters,
    };

    let mut buf = subscribe_update.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeUpdate as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = SubscribeUpdate::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
