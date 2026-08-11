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
use crate::model::common::tuple::{Tuple, TupleField};
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::data::full_track_name::FullTrackName;
use crate::model::error::ParseError;
use crate::model::parameter::message_parameter::{
  MessageParameter, deserialize_message_parameters, serialize_message_parameters,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};

/// A TRACK_STATUS has the same shape as a SUBSCRIBE. It asks for a track's properties
/// without subscribing, so parameters that govern delivery do not belong on it.
#[derive(Debug, PartialEq, Clone)]
pub struct TrackStatus {
  pub request_id: u64,
  pub track_namespace: Tuple,
  pub track_name: TupleField,
  pub subscribe_parameters: Vec<MessageParameter>,
}

impl TrackStatus {
  pub fn new(
    request_id: u64,
    track_namespace: Tuple,
    track_name: TupleField,
    subscribe_parameters: Vec<MessageParameter>,
  ) -> Self {
    Self {
      request_id,
      track_namespace,
      track_name,
      subscribe_parameters,
    }
  }

  pub fn get_full_track_name(&self) -> FullTrackName {
    FullTrackName {
      namespace: self.track_namespace.clone(),
      name: self.track_name.clone(),
    }
  }
}

impl ControlMessageTrait for TrackStatus {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::TrackStatus)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.extend_from_slice(&self.track_namespace.serialize()?);
    payload.put_vi(self.track_name.len())?;
    payload.extend_from_slice(self.track_name.as_bytes());
    payload.put_vi(self.subscribe_parameters.len())?;
    payload.extend_from_slice(&serialize_message_parameters(&self.subscribe_parameters)?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "TrackStatus::serialize",
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

    let name_len_u64 = payload.get_vi()?;
    let name_len: usize = name_len_u64
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "TrackStatus::parse_payload(track_name_len)",
        from_type: "u64",
        to_type: "usize",
        details: e.to_string(),
      })?;

    if payload.remaining() < name_len {
      return Err(ParseError::NotEnoughBytes {
        context: "TrackStatus::parse_payload(track_name)",
        needed: name_len,
        available: payload.remaining(),
      });
    }
    let track_name = TupleField::new(payload.copy_to_bytes(name_len));

    let param_count = payload.get_vi()?;
    let subscribe_parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::TrackStatus)?;

    Ok(Box::new(TrackStatus {
      request_id,
      track_namespace,
      track_name,
      subscribe_parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::TrackStatus
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::parameter::authorization_token::AuthorizationToken;
  use bytes::Buf;

  fn sample() -> TrackStatus {
    TrackStatus::new(
      128,
      Tuple::from_utf8_path("un/deux/trois"),
      TupleField::from_utf8("quatre"),
      vec![MessageParameter::new_authorization_token(
        AuthorizationToken::new_use_alias(42),
      )],
    )
  }

  #[test]
  fn test_roundtrip() {
    let track_status = sample();
    let mut buf = track_status.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::TrackStatus as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = TrackStatus::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, track_status);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let track_status = sample();
    let serialized = track_status.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::TrackStatus as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = TrackStatus::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, track_status);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let track_status = sample();
    let mut buf = track_status.serialize().unwrap();
    let _ = buf.get_vi().unwrap();
    let _ = buf.get_u16();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    assert!(TrackStatus::parse_payload(&mut partial).is_err());
  }

  /// The body is a SUBSCRIBE body: the delivery fields draft-16 carried inline are gone.
  #[test]
  fn body_matches_subscribe() {
    use super::super::subscribe::Subscribe;
    let track_status = sample();
    let subscribe = Subscribe::new(
      track_status.request_id,
      track_status.track_namespace.clone(),
      track_status.track_name.clone(),
      track_status.subscribe_parameters.clone(),
    );

    let mut ts = track_status.serialize().unwrap();
    let mut sub = subscribe.serialize().unwrap();
    let _ = ts.get_vi().unwrap();
    let _ = sub.get_vi().unwrap();
    assert_eq!(ts.get_u16(), sub.get_u16());
    assert_eq!(ts.chunk(), sub.chunk());
  }
}
