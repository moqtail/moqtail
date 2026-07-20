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
  MessageParameter, deserialize_message_parameters, serialize_message_parameters,
};
use crate::model::property::track_property::{
  TrackProperty, deserialize_track_properties, serialize_track_properties,
};
use bytes::{BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct SubscribeOk {
  pub track_alias: u64,
  pub subscribe_parameters: Vec<MessageParameter>,
  pub track_properties: Vec<TrackProperty>,
}

impl SubscribeOk {
  pub fn new(
    track_alias: u64,
    subscribe_parameters: Vec<MessageParameter>,
    track_properties: Vec<TrackProperty>,
  ) -> Self {
    Self {
      track_alias,
      subscribe_parameters,
      track_properties,
    }
  }
}

impl ControlMessageTrait for SubscribeOk {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::SubscribeOk)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.track_alias)?;

    payload.put_vi(self.subscribe_parameters.len() as u64)?;
    payload.extend_from_slice(&serialize_message_parameters(&self.subscribe_parameters)?);

    // Track Properties (no length prefix; bounded by outer message Length field)
    payload.extend_from_slice(&serialize_track_properties(&self.track_properties)?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "SubscribeOk::serialize(payload_length)",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;

    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);

    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let track_alias = payload.get_vi()?;

    let param_count = payload.get_vi()?;
    let subscribe_parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::SubscribeOk)?;

    // Track Properties: consume whatever remains in the payload
    let track_properties = deserialize_track_properties(payload)?;

    Ok(Box::new(SubscribeOk {
      track_alias,
      subscribe_parameters,
      track_properties,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::SubscribeOk
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::location::Location;
  use crate::model::control::constant::GroupOrder;
  use crate::model::property::track_property::TrackProperty;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let mut subscribe_ok = SubscribeOk {
      track_alias: 0,
      subscribe_parameters: vec![
        MessageParameter::new_expires(16),
        MessageParameter::new_group_order(GroupOrder::Ascending),
        MessageParameter::new_largest_object(Location {
          group: 34,
          object: 0,
        }),
        MessageParameter::new_expires(100),
      ],
      track_properties: vec![],
    };
    // Wire encoding canonicalizes parameter order ascending by type (delta-encoding requirement).
    subscribe_ok
      .subscribe_parameters
      .sort_by_key(|p| p.type_value());

    let mut buf = subscribe_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = SubscribeOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe_ok);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_with_track_properties() {
    let subscribe_ok = SubscribeOk {
      track_alias: 42,
      subscribe_parameters: vec![
        MessageParameter::new_expires(0),
        MessageParameter::new_group_order(GroupOrder::Descending),
      ],
      track_properties: vec![
        TrackProperty::ObjectDeliveryTimeout { timeout_ms: 500 },
        TrackProperty::DynamicGroups { enabled: true },
      ],
    };

    let mut buf = subscribe_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = SubscribeOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe_ok);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let subscribe_ok = SubscribeOk {
      track_alias: 89123u64,
      subscribe_parameters: vec![
        MessageParameter::new_expires(16),
        MessageParameter::new_largest_object(Location {
          group: 34,
          object: 0,
        }),
      ],
      track_properties: vec![],
    };

    let serialized = subscribe_ok.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeOk as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let mut payload = buf.copy_to_bytes(msg_length as usize);
    let deserialized = SubscribeOk::parse_payload(&mut payload).unwrap();
    assert_eq!(*deserialized, subscribe_ok);
    assert!(!payload.has_remaining());
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let subscribe_ok = SubscribeOk {
      track_alias: 1223u64,
      subscribe_parameters: vec![
        MessageParameter::new_expires(16),
        MessageParameter::new_largest_object(Location {
          group: 34,
          object: 0,
        }),
      ],
      track_properties: vec![],
    };
    let mut buf = subscribe_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = SubscribeOk::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
