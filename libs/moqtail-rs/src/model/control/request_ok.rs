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
  MessageParameter, deserialize_message_parameters, serialize_message_parameters,
};
use crate::model::property::track_property::{
  TrackProperty, deserialize_track_properties, serialize_track_properties,
};
use bytes::{BufMut, Bytes, BytesMut};

/// REQUEST_OK (0x7) answers PUBLISH, REQUEST_UPDATE, TRACK_STATUS,
/// SUBSCRIBE_NAMESPACE, SUBSCRIBE_TRACKS and PUBLISH_NAMESPACE. There is one wire
/// type; the per-request-type names (PUBLISH_OK, TRACK_STATUS_OK, ...) are
/// shorthands, not distinct messages.
#[derive(Debug, PartialEq, Clone)]
pub struct RequestOk {
  pub parameters: Vec<MessageParameter>,
  /// Populated only in TRACK_STATUS_OK; empty for every other request type.
  pub track_properties: Vec<TrackProperty>,
}

impl RequestOk {
  /// A REQUEST_OK with no Track Properties (every request type except TRACK_STATUS).
  pub fn new(parameters: Vec<MessageParameter>) -> Self {
    Self {
      parameters,
      track_properties: Vec::new(),
    }
  }

  /// A TRACK_STATUS_OK carrying Track Properties.
  pub fn new_track_status(
    parameters: Vec<MessageParameter>,
    track_properties: Vec<TrackProperty>,
  ) -> Self {
    Self {
      parameters,
      track_properties,
    }
  }

  /// Track Properties may only be non-empty when this REQUEST_OK answers a
  /// TRACK_STATUS request. A receiver that sees them in any other REQUEST_OK
  /// (PUBLISH_OK, REQUEST_UPDATE_OK, SUBSCRIBE_NAMESPACE_OK, PUBLISH_NAMESPACE_OK)
  /// MUST close the session with a PROTOCOL_VIOLATION.
  pub fn validate_track_properties(&self, answers_track_status: bool) -> Result<(), ParseError> {
    if !answers_track_status && !self.track_properties.is_empty() {
      return Err(ParseError::ProtocolViolation {
        context: "RequestOk::validate_track_properties",
        details: "Track Properties present in a non-TRACK_STATUS REQUEST_OK".to_string(),
      });
    }
    Ok(())
  }
}

impl ControlMessageTrait for RequestOk {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::RequestOk)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.parameters.len() as u64)?;
    payload.extend_from_slice(&serialize_message_parameters(&self.parameters)?);

    // Track Properties span the remaining message length (no explicit count).
    payload.extend_from_slice(&serialize_track_properties(&self.track_properties)?);

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
    let param_count = payload.get_vi()?;
    let parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::RequestOk)?;

    // Whatever remains is the Track Properties sequence.
    let track_properties = deserialize_track_properties(payload)?;

    Ok(Box::new(RequestOk {
      parameters,
      track_properties,
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
    let request_ok = RequestOk::new(vec![]);

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
    let request_ok = RequestOk::new(vec![
      MessageParameter::new_expires(3600),
      MessageParameter::new_group_order(GroupOrder::Ascending),
    ]);

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
    let request_ok = RequestOk::new(vec![MessageParameter::new_expires(3600)]);

    let serialized = request_ok.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::RequestOk as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let mut payload = buf.copy_to_bytes(msg_length as usize);
    let deserialized = RequestOk::parse_payload(&mut payload).unwrap();
    assert_eq!(*deserialized, request_ok);
    assert!(!payload.has_remaining());
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let request_ok = RequestOk::new(vec![MessageParameter::new_expires(3600)]);
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

  #[test]
  fn test_roundtrip_track_status_with_track_properties() {
    use crate::model::property::track_property::TrackProperty;
    let request_ok = RequestOk::new_track_status(
      vec![MessageParameter::new_expires(60)],
      vec![
        TrackProperty::MaxCacheDuration {
          duration_ms: 60_000,
        },
        TrackProperty::DefaultPublisherPriority { priority: 3 },
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
  fn test_track_properties_only_valid_for_track_status() {
    use crate::model::property::track_property::TrackProperty;
    let with_props = RequestOk::new_track_status(
      vec![],
      vec![TrackProperty::MaxCacheDuration { duration_ms: 1 }],
    );
    // Non-empty properties answering a TRACK_STATUS request are allowed.
    assert!(with_props.validate_track_properties(true).is_ok());
    // Non-empty properties answering any other request type are a violation.
    assert!(matches!(
      with_props.validate_track_properties(false),
      Err(ParseError::ProtocolViolation { .. })
    ));

    // Empty properties are always fine.
    let empty = RequestOk::new(vec![]);
    assert!(empty.validate_track_properties(false).is_ok());
    assert!(empty.validate_track_properties(true).is_ok());
  }
}
