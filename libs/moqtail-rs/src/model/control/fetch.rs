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
use crate::model::data::full_track_name::FullTrackName;
use crate::model::error::ParseError;
use crate::model::parameter::message_parameter::{
  MessageParameter, deserialize_message_parameters, serialize_message_parameters,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct Fetch {
  pub request_id: u64,
  pub track_namespace: Tuple,
  pub track_name: TupleField,
  pub start_location: Location,
  /// The last Object plus 1. An Object value of 0 means the entire group.
  pub end_location: Location,
  pub parameters: Vec<MessageParameter>,
}

impl Fetch {
  pub fn new(
    request_id: u64,
    track_namespace: Tuple,
    track_name: TupleField,
    start_location: Location,
    end_location: Location,
    parameters: Vec<MessageParameter>,
  ) -> Self {
    Self {
      request_id,
      track_namespace,
      track_name,
      start_location,
      end_location,
      parameters,
    }
  }

  pub fn get_full_track_name(&self) -> FullTrackName {
    FullTrackName {
      namespace: self.track_namespace.clone(),
      name: self.track_name.clone(),
    }
  }

  pub fn group_order(&self) -> GroupOrder {
    self
      .parameters
      .iter()
      .find_map(|p| match p {
        MessageParameter::GroupOrder { order } => Some(*order),
        _ => None,
      })
      .unwrap_or(GroupOrder::Ascending)
  }
}

impl ControlMessageTrait for Fetch {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::Fetch as u64)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.extend_from_slice(&self.track_namespace.serialize()?);
    payload.put_vi(self.track_name.len())?;
    payload.extend_from_slice(self.track_name.as_bytes());
    payload.extend_from_slice(&self.start_location.serialize()?);
    payload.extend_from_slice(&self.end_location.serialize()?);

    payload.put_vi(self.parameters.len())?;
    payload.extend_from_slice(&serialize_message_parameters(&self.parameters)?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "Fetch::serialize(payload_length)",
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

    let track_name_len = payload.get_vi()? as usize;
    if payload.remaining() < track_name_len {
      return Err(ParseError::NotEnoughBytes {
        context: "Fetch::parse_payload(track_name)",
        needed: track_name_len,
        available: payload.remaining(),
      });
    }
    let track_name = TupleField::new(payload.copy_to_bytes(track_name_len));

    let start_location = Location::deserialize(payload)?;
    let end_location = Location::deserialize(payload)?;

    let param_count = payload.get_vi()?;
    let parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::Fetch)?;

    Ok(Box::new(Fetch {
      request_id,
      track_namespace,
      track_name,
      start_location,
      end_location,
      parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::Fetch
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use crate::model::{
    common::tuple::TupleField, control::constant::GroupOrder,
    parameter::authorization_token::AuthorizationToken,
  };
  use bytes::Buf;

  fn sample_fetch() -> Fetch {
    Fetch::new(
      161803,
      Tuple::from_utf8_path("un/deux/trois"),
      TupleField::from_utf8("quatre"),
      Location::new(12, 5),
      Location::new(20, 0),
      vec![
        MessageParameter::new_authorization_token(AuthorizationToken::new_use_value(
          0,
          Bytes::from_static(b"test-token"),
        )),
        MessageParameter::new_subscriber_priority(42),
        MessageParameter::new_group_order(GroupOrder::Ascending),
      ],
    )
  }

  #[test]
  fn test_roundtrip() {
    let fetch = sample_fetch();

    let mut buf = fetch.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Fetch as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = Fetch::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, fetch);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let fetch = sample_fetch();

    let serialized = fetch.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Fetch as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = Fetch::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, fetch);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let fetch = sample_fetch();

    let mut buf = fetch.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Fetch as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = Fetch::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }

  #[test]
  fn test_group_order_defaults_to_ascending() {
    let fetch = Fetch::new(
      1,
      Tuple::from_utf8_path("ns"),
      TupleField::from_utf8("track"),
      Location::new(0, 0),
      Location::new(1, 0),
      vec![],
    );
    assert_eq!(fetch.group_order(), GroupOrder::Ascending);
  }
}
