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
use bytes::{BufMut, Bytes, BytesMut};

/// PUBLISH_STATE_NOTIFY (0x22): the publisher telling the subscriber that a
/// subscription's state changed for some reason other than the subscriber's own
/// REQUEST_UPDATE (§10.10).
///
/// It carries no Request ID because it rides the subscription's own bidirectional
/// stream, and it carries only the parameters whose values changed; an absent one is
/// unchanged.
///
/// Unlike REQUEST_UPDATE it is **unilateral**: the receiver sends no REQUEST_OK or
/// REQUEST_ERROR, and it is not counted against MAX_REQUEST_UPDATES. It is
/// informative, and no action is required of the recipient.
///
/// It applies only to subscriptions and travels only publisher to subscriber.
/// Receiving one for any other request type, or from the subscriber, is a
/// PROTOCOL_VIOLATION that closes the session -- a direction the codec cannot see, so
/// it is enforced where the stream's role is known.
#[derive(Debug, PartialEq, Clone)]
pub struct PublishStateNotify {
  pub parameters: Vec<MessageParameter>,
}

impl PublishStateNotify {
  pub fn new(parameters: Vec<MessageParameter>) -> Self {
    Self { parameters }
  }
}

impl ControlMessageTrait for PublishStateNotify {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::PublishStateNotify)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.parameters.len())?;
    payload.extend_from_slice(&serialize_message_parameters(&self.parameters)?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "PublishStateNotify::serialize(payload_length)",
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
      deserialize_message_parameters(payload, param_count, ControlMessageType::PublishStateNotify)?;

    Ok(Box::new(PublishStateNotify { parameters }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::PublishStateNotify
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::location::Location;
  use bytes::Buf;

  fn sample() -> PublishStateNotify {
    PublishStateNotify::new(vec![
      MessageParameter::new_largest_object(Location::new(81, 7)),
      MessageParameter::new_forward(false),
    ])
  }

  #[test]
  fn test_roundtrip() {
    let msg = sample();
    let mut buf = msg.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishStateNotify as u64);
    assert_eq!(msg_type, 0x22);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = PublishStateNotify::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, msg);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let msg = sample();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&msg.serialize().unwrap());
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishStateNotify as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = PublishStateNotify::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, msg);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let mut buf = sample().serialize().unwrap();
    let _ = buf.get_vi().unwrap();
    let _ = buf.get_u16();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    assert!(PublishStateNotify::parse_payload(&mut partial).is_err());
  }

  #[test]
  fn a_notify_carrying_nothing_round_trips() {
    // "If a parameter is not present, its value is unchanged", so an empty one says
    // nothing changed rather than being malformed.
    let msg = PublishStateNotify::new(vec![]);
    let mut buf = msg.serialize().unwrap();
    let _ = buf.get_vi().unwrap();
    let _ = buf.get_u16();
    assert_eq!(*PublishStateNotify::parse_payload(&mut buf).unwrap(), msg);
  }

  #[test]
  fn only_the_three_parameters_the_draft_names_are_accepted() {
    // §10.2.9, §10.2.17 and §10.2.18 each name PUBLISH_STATE_NOTIFY; no other
    // parameter does, and is_valid_for is what refuses the rest.
    for allowed in [
      MessageParameter::new_largest_object(Location::new(1, 2)),
      MessageParameter::new_forward(true),
      MessageParameter::new_location_filter_next_group(),
    ] {
      assert!(
        allowed.is_valid_for(ControlMessageType::PublishStateNotify),
        "{allowed:?} is named for PUBLISH_STATE_NOTIFY"
      );
    }
    for refused in [
      MessageParameter::new_subscriber_priority(7),
      MessageParameter::new_group_request(3),
      MessageParameter::new_object_delivery_timeout(100),
    ] {
      assert!(
        !refused.is_valid_for(ControlMessageType::PublishStateNotify),
        "{refused:?} is not"
      );
    }
  }
}
