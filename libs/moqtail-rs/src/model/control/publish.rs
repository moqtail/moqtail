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

use super::constant::{ControlMessageType, GroupOrder};
use super::control_message::ControlMessageTrait;
use crate::model::common::location::Location;
use crate::model::common::tuple::{Tuple, TupleField};
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use crate::model::extension_header::track_extension::{
  TrackExtension, deserialize_track_extensions, serialize_track_extensions,
};
use crate::model::parameter::message_parameter::{
  MessageParameter, deserialize_message_parameters,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct Publish {
  pub request_id: u64,
  pub track_namespace: Tuple,
  pub track_name: TupleField,
  pub track_alias: u64,
  pub group_order: GroupOrder,
  pub content_exists: u8,
  pub largest_location: Option<Location>,
  pub forward: u8,
  pub parameters: Vec<MessageParameter>,
  pub track_extensions: Vec<TrackExtension>,
}

#[allow(clippy::too_many_arguments)]
impl Publish {
  pub fn new(
    request_id: u64,
    track_namespace: Tuple,
    track_name: TupleField,
    track_alias: u64,
    group_order: GroupOrder,
    content_exists: u8,
    largest_location: Option<Location>,
    forward: u8,
    parameters: Vec<MessageParameter>,
    track_extensions: Vec<TrackExtension>,
  ) -> Self {
    Self {
      request_id,
      track_namespace,
      track_name,
      track_alias,
      group_order,
      content_exists,
      largest_location,
      forward,
      parameters,
      track_extensions,
    }
  }
}

impl ControlMessageTrait for Publish {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::Publish)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.extend_from_slice(&self.track_namespace.serialize()?);

    // Track Name Length and Track Name
    payload.put_vi(self.track_name.len() as u64)?;
    payload.extend_from_slice(self.track_name.as_bytes());

    payload.put_vi(self.track_alias)?;
    payload.put_u8(self.group_order.into());
    payload.put_u8(self.content_exists);

    // Largest Location is conditional
    if self.content_exists == 1 {
      if let Some(ref location) = self.largest_location {
        payload.extend_from_slice(&location.serialize()?);
      } else {
        return Err(ParseError::ProtocolViolation {
          context: "Publish::serialize",
          details: "content_exists is 1 but largest_location is None".to_string(),
        });
      }
    }

    payload.put_u8(self.forward);

    // Parameters
    payload.put_vi(self.parameters.len() as u64)?;
    for param in &self.parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    // Track Extensions (no length prefix; bounded by outer message Length field)
    payload.extend_from_slice(&serialize_track_extensions(&self.track_extensions)?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "Publish::serialize(payload_length)",
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

    let track_name_length = payload.get_vi()? as usize;
    if payload.remaining() < track_name_length {
      return Err(ParseError::NotEnoughBytes {
        context: "Publish::parse_payload(track_name)",
        needed: track_name_length,
        available: payload.remaining(),
      });
    }
    let track_name = TupleField::new(payload.copy_to_bytes(track_name_length));

    let track_alias = payload.get_vi()?;
    let group_order = GroupOrder::try_from(payload.get_u8())?;
    let content_exists = payload.get_u8();

    if content_exists != 0 && content_exists != 1 {
      return Err(ParseError::ProtocolViolation {
        context: "Publish::parse_payload(content_exists)",
        details: format!("content_exists must be 0 or 1, got {}", content_exists),
      });
    }

    let largest_location = if content_exists == 1 {
      Some(Location::deserialize(payload)?)
    } else {
      None
    };

    let forward = payload.get_u8();
    if forward != 0 && forward != 1 {
      return Err(ParseError::ProtocolViolation {
        context: "Publish::parse_payload(forward)",
        details: format!("forward must be 0 or 1, got {}", forward),
      });
    }

    let param_count = payload.get_vi()?;
    let parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::Publish)?;

    // Track Extensions: consume whatever remains in the payload
    let track_extensions = deserialize_track_extensions(payload)?;

    Ok(Box::new(Publish {
      request_id,
      track_namespace,
      track_name,
      track_alias,
      group_order,
      content_exists,
      largest_location,
      forward,
      parameters,
      track_extensions,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::Publish
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::location::Location;
  use crate::model::parameter::message_parameter::MessageParameter;
  use bytes::Buf;

  #[test]
  fn test_roundtrip_with_content() {
    let request_id = 123;
    let track_namespace = Tuple::from_utf8_path("example/track");
    let track_name = TupleField::from_utf8("video");
    let track_alias = 456;
    let group_order = GroupOrder::Ascending;
    let content_exists = 1;
    let largest_location = Some(Location::new(10, 20));
    let forward = 1;
    let parameters = vec![MessageParameter::new_expires(1000)];
    let track_extensions = vec![];

    let publish = Publish::new(
      request_id,
      track_namespace,
      track_name,
      track_alias,
      group_order,
      content_exists,
      largest_location,
      forward,
      parameters,
      track_extensions,
    );

    let mut buf = publish.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Publish as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = Publish::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, publish);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_without_content() {
    let request_id = 123;
    let track_namespace = Tuple::from_utf8_path("example/track");
    let track_name = TupleField::from_utf8("video");
    let track_alias = 456;
    let group_order = GroupOrder::Ascending;
    let content_exists = 0;
    let largest_location = None;
    let forward = 0;
    let parameters = vec![];
    let track_extensions = vec![];

    let publish = Publish::new(
      request_id,
      track_namespace,
      track_name,
      track_alias,
      group_order,
      content_exists,
      largest_location,
      forward,
      parameters,
      track_extensions,
    );

    let mut buf = publish.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Publish as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = Publish::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, publish);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_with_track_extensions() {
    let publish = Publish::new(
      1,
      Tuple::from_utf8_path("ns/track"),
      TupleField::from_utf8("audio"),
      7,
      GroupOrder::Ascending,
      0,
      None,
      0,
      vec![],
      vec![
        TrackExtension::DeliveryTimeout { timeout_ms: 3000 },
        TrackExtension::DefaultPublisherPriority { priority: 100 },
      ],
    );

    let mut buf = publish.serialize().unwrap();
    let _msg_type = buf.get_vi().unwrap();
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = Publish::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, publish);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_invalid_content_exists() {
    let publish = Publish::new(
      123,
      Tuple::from_utf8_path("example/track"),
      TupleField::from_utf8("video"),
      456,
      GroupOrder::Ascending,
      1,
      None, // Should have location when content_exists = 1
      0,
      vec![],
      vec![],
    );

    let result = publish.serialize();
    assert!(result.is_err());
  }
}
