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
use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::convert::TryInto;

#[derive(Debug, PartialEq, Clone)]
pub struct SubscribeOk {
  pub request_id: u64,
  pub track_alias: u64,
  pub expires: u64,
  pub group_order: GroupOrder, // Must be Ascending or Descending
  pub content_exists: bool,
  pub largest_location: Option<Location>, // Present only if content_exists is true
  pub subscribe_parameters: Option<Vec<KeyValuePair>>,
}

impl SubscribeOk {
  pub fn new_ascending_no_content(
    request_id: u64,
    track_alias: u64,
    expires: u64,
    subscribe_parameters: Option<Vec<KeyValuePair>>,
  ) -> Self {
    Self {
      request_id,
      track_alias,
      expires,
      group_order: GroupOrder::Ascending,
      content_exists: false,
      largest_location: None,
      subscribe_parameters,
    }
  }

  pub fn new_descending_no_content(
    request_id: u64,
    track_alias: u64,
    expires: u64,
    subscribe_parameters: Option<Vec<KeyValuePair>>,
  ) -> Self {
    Self {
      request_id,
      track_alias,
      expires,
      group_order: GroupOrder::Descending,
      content_exists: false,
      largest_location: None,
      subscribe_parameters,
    }
  }

  pub fn new_ascending_with_content(
    request_id: u64,
    track_alias: u64,
    expires: u64,
    largest_location: Option<Location>,
    subscribe_parameters: Option<Vec<KeyValuePair>>,
  ) -> Self {
    Self {
      request_id,
      track_alias,
      expires,
      group_order: GroupOrder::Ascending,
      content_exists: true,
      largest_location,
      subscribe_parameters,
    }
  }

  pub fn new_descending_with_content(
    request_id: u64,
    track_alias: u64,
    expires: u64,
    largest_location: Option<Location>,
    subscribe_parameters: Option<Vec<KeyValuePair>>,
  ) -> Self {
    Self {
      request_id,
      track_alias,
      expires,
      group_order: GroupOrder::Descending,
      content_exists: true,
      largest_location,
      subscribe_parameters,
    }
  }
}
impl ControlMessageTrait for SubscribeOk {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::SubscribeOk)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.put_vi(self.track_alias)?;
    payload.put_vi(self.expires)?;

    if self.group_order == GroupOrder::Original {
      unreachable!()
    }

    payload.put_u8(self.group_order as u8);

    if self.content_exists {
      payload.put_u8(1u8);
      if let Some(ref loc) = self.largest_location {
        payload.extend_from_slice(&loc.serialize()?);
      } else {
        // default
        payload.extend_from_slice(&Location::new(0, 0).serialize()?);
      }
    } else {
      payload.put_u8(0u8);
    }

    if let Some(ref params) = self.subscribe_parameters {
      payload.put_vi(params.len() as u64)?;
      for param in params {
        payload.extend_from_slice(&param.serialize()?);
      }
    } else {
      payload.put_vi(0u64)?;
    }

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
    let request_id = payload.get_vi()?;
    let track_alias = payload.get_vi()?;
    let expires = payload.get_vi()?;

    if payload.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "SubscribeOk::parse_payload(group_order)",
        needed: 1,
        available: payload.remaining(),
      });
    }
    let group_order_raw = payload.get_u8();
    let group_order = GroupOrder::try_from(group_order_raw)?;
    if let GroupOrder::Original = group_order {
      return Err(ParseError::ProtocolViolation {
        context: "SubscribeOk::parse_payload(group_order_validation)",
        details: "Group order must be Ascending(0x01) or Descending(0x02)".to_string(),
      });
    }
    if payload.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "SubscribeOk::parse_payload(content_exists)",
        needed: 1,
        available: payload.remaining(),
      });
    }
    let content_exists_raw = payload.get_u8();
    let content_exists = match content_exists_raw {
      0 => false,
      1 => true,
      _ => {
        return Err(ParseError::ProtocolViolation {
          context: "SubscribeOk::parse_payload(content_exists)",
          details: format!("Invalid Content Exists value: {content_exists_raw}"),
        });
      }
    };

    let mut largest_location = None;
    if content_exists {
      largest_location = Some(Location::deserialize(payload)?);
    }

    let param_count_u64 = payload.get_vi()?;
    let param_count: usize =
      param_count_u64
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "SubscribeOk::parse_payload(param_count)",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;

    let subscribe_parameters = if param_count > 0 {
      let mut params = Vec::with_capacity(param_count);
      for _ in 0..param_count {
        let param = KeyValuePair::deserialize(payload)?;
        params.push(param);
      }
      Some(params)
    } else {
      None
    };

    Ok(Box::new(SubscribeOk {
      request_id,
      track_alias,
      expires,
      group_order,
      content_exists,
      largest_location,
      subscribe_parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::SubscribeOk
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let request_id = 145136;
    let track_alias = 0;
    let expires = 16;
    let group_order = GroupOrder::Ascending;
    let content_exists = true;
    let largest_location = Location {
      group: 34,
      object: 0,
    };
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"9 gifted subs from Dr.Doofishtein"))
        .unwrap(),
    ];
    let subscribe_ok = SubscribeOk {
      request_id,
      track_alias,
      expires,
      group_order,
      content_exists,
      largest_location: Some(largest_location),
      subscribe_parameters: Some(subscribe_parameters),
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
    let request_id = 145136;
    let track_alias = 89123u64;
    let expires = 16;
    let group_order = GroupOrder::Ascending;
    let content_exists = true;
    let largest_location = Location {
      group: 34,
      object: 0,
    };
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"9 gifted subs from Dr.Doofishtein"))
        .unwrap(),
    ];
    let subscribe_ok = SubscribeOk {
      request_id,
      track_alias,
      expires,
      group_order,
      content_exists,
      largest_location: Some(largest_location),
      subscribe_parameters: Some(subscribe_parameters),
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
    let deserialized = SubscribeOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe_ok);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let request_id = 145136;
    let track_alias = 1223u64;
    let expires = 16;
    let group_order = GroupOrder::Ascending;
    let content_exists = true;
    let largest_location = Location {
      group: 34,
      object: 0,
    };
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"9 gifted subs from Dr.Doofishtein"))
        .unwrap(),
    ];
    let subscribe_ok = SubscribeOk {
      request_id,
      track_alias,
      expires,
      group_order,
      content_exists,
      largest_location: Some(largest_location),
      subscribe_parameters: Some(subscribe_parameters),
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
