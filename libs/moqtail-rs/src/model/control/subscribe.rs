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

use super::constant::{ControlMessageType, FilterType, GroupOrder};
use super::control_message::ControlMessageTrait;
use crate::model::common::location::Location;
use crate::model::common::pair::KeyValuePair;
use crate::model::common::tuple::Tuple;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::data::full_track_name::FullTrackName;
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct Subscribe {
  pub request_id: u64,
  pub track_namespace: Tuple,
  pub track_name: String,
  pub subscriber_priority: u8,
  pub group_order: GroupOrder,
  pub forward: bool,
  pub filter_type: FilterType,
  pub start_location: Option<Location>,
  pub end_group: Option<u64>,
  // TODO: make the following optional
  pub subscribe_parameters: Vec<KeyValuePair>,
}

#[allow(clippy::too_many_arguments)]
impl Subscribe {
  pub fn new_next_group_start(
    request_id: u64,
    track_namespace: Tuple,
    track_name: String,
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
    subscribe_parameters: Vec<KeyValuePair>,
  ) -> Self {
    Self {
      request_id,
      track_namespace,
      track_name,
      subscriber_priority,
      group_order,
      forward,
      filter_type: FilterType::NextGroupStart,
      start_location: None,
      end_group: None,
      subscribe_parameters,
    }
  }

  pub fn new_latest_object(
    request_id: u64,
    track_namespace: Tuple,
    track_name: String,
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
    subscribe_parameters: Vec<KeyValuePair>,
  ) -> Self {
    Self {
      request_id,
      track_namespace,
      track_name,
      subscriber_priority,
      group_order,
      forward,
      filter_type: FilterType::LatestObject,
      start_location: None,
      end_group: None,
      subscribe_parameters,
    }
  }

  pub fn new_absolute_start(
    request_id: u64,
    track_namespace: Tuple,
    track_name: String,
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
    start_location: Location,
    subscribe_parameters: Vec<KeyValuePair>,
  ) -> Self {
    Self {
      request_id,
      track_namespace,
      track_name,
      subscriber_priority,
      group_order,
      forward,
      filter_type: FilterType::AbsoluteStart,
      start_location: Some(start_location),
      end_group: None,
      subscribe_parameters,
    }
  }

  pub fn new_absolute_range(
    request_id: u64,
    track_namespace: Tuple,
    track_name: String,
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
    start_location: Location,
    end_group: u64,
    subscribe_parameters: Vec<KeyValuePair>,
  ) -> Self {
    assert!(
      end_group >= start_location.group,
      "End Group must be >= Start Group"
    );
    Self {
      request_id,
      track_namespace,
      track_name,
      subscriber_priority,
      group_order,
      forward,
      filter_type: FilterType::AbsoluteRange,
      start_location: Some(start_location),
      end_group: Some(end_group),
      subscribe_parameters,
    }
  }

  pub fn get_full_track_name(&self) -> FullTrackName {
    FullTrackName {
      namespace: self.track_namespace.clone(),
      name: self.track_name.clone().into(),
    }
  }
}
impl ControlMessageTrait for Subscribe {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::Subscribe)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;

    payload.extend_from_slice(&self.track_namespace.serialize()?);
    payload.put_vi(self.track_name.len())?;
    payload.extend_from_slice(self.track_name.as_bytes());
    payload.put_u8(self.subscriber_priority);
    payload.put_u8(self.group_order as u8);
    payload.put_u8(if self.forward { 1 } else { 0 });
    payload.put_vi(self.filter_type)?;

    match self.filter_type {
      FilterType::AbsoluteStart => {
        if let Some(ref loc) = self.start_location {
          payload.extend_from_slice(&loc.serialize()?);
        } else {
          unreachable!()
        }
      }
      FilterType::AbsoluteRange => {
        if let Some(ref loc) = self.start_location {
          payload.extend_from_slice(&loc.serialize()?);
        }
        if let Some(eg) = self.end_group {
          payload.put_vi(eg)?;
        }
      }
      _ => {}
    }

    payload.put_vi(self.subscribe_parameters.len())?;
    for param in &self.subscribe_parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "Subscribe::serialize",
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
        context: "Subscribe::parse_payload(track_name_len)",
        from_type: "u64",
        to_type: "usize",
        details: e.to_string(),
      })?;

    if payload.remaining() < name_len {
      return Err(ParseError::NotEnoughBytes {
        context: "Subscribe::parse_payload(track_name)",
        needed: name_len,
        available: payload.remaining(),
      });
    }
    let track_name_bytes = payload.copy_to_bytes(name_len);
    let track_name =
      String::from_utf8(track_name_bytes.to_vec()).map_err(|e| ParseError::InvalidUTF8 {
        context: "Subscribe::parse_payload(track_name)",
        details: e.to_string(),
      })?;

    if payload.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "Subscribe::parse_payload(subscriber_priority)",
        needed: 1,
        available: 0,
      });
    }
    let subscriber_priority = payload.get_u8();

    if payload.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "Subscribe::parse_payload(group_order)",
        needed: 1,
        available: 0,
      });
    }
    let group_order_raw = payload.get_u8();
    let group_order = GroupOrder::try_from(group_order_raw)?;

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

    let filter_type_raw = payload.get_vi()?;
    let filter_type = FilterType::try_from(filter_type_raw)?;

    let mut start_location: Option<Location> = None;
    let mut end_group: Option<u64> = None;

    match filter_type {
      FilterType::AbsoluteRange => {
        start_location = Some(Location::deserialize(payload)?);
        end_group = Some(payload.get_vi()?);
      }
      FilterType::AbsoluteStart => {
        start_location = Some(Location::deserialize(payload)?);
      }
      FilterType::LatestObject => {}
      FilterType::NextGroupStart => {}
    }

    let param_count_u64 = payload.get_vi()?;
    let param_count: usize =
      param_count_u64
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "Subscribe::deserialize(param_count)",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;

    let mut subscribe_parameters = Vec::with_capacity(param_count);
    for _ in 0..param_count {
      let param = KeyValuePair::deserialize(payload)?;
      subscribe_parameters.push(param);
    }

    Ok(Box::new(Subscribe {
      request_id,
      track_namespace,
      track_name,
      subscriber_priority,
      group_order,
      forward,
      filter_type,
      start_location,
      end_group,
      subscribe_parameters,
    }))
  }
  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::Subscribe
  }
}
#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let request_id = 128242;
    let track_namespace = Tuple::from_utf8_path("nein/nein/nein");
    let track_name = "${Name}".to_string();
    let subscriber_priority = 31;
    let group_order = GroupOrder::Original;
    let forward = true;
    let filter_type = FilterType::AbsoluteRange;
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let end_group = 25;
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"I'll sync you up")).unwrap(),
    ];
    let subscribe = Subscribe {
      request_id,
      track_namespace,
      track_name,
      subscriber_priority,
      group_order,
      forward,
      filter_type,
      start_location: Some(start_location),
      end_group: Some(end_group),
      subscribe_parameters,
    };

    let mut buf = subscribe.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Subscribe as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = Subscribe::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let request_id = 128242;
    let track_namespace = Tuple::from_utf8_path("nein/nein/nein");
    let track_name = "${Name}".to_string();
    let subscriber_priority = 31;
    let group_order = GroupOrder::Original;
    let forward = true;
    let filter_type = FilterType::AbsoluteRange;
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let end_group = 25;
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"I'll sync you up")).unwrap(),
    ];
    let subscribe = Subscribe {
      request_id,
      track_namespace,
      track_name,
      subscriber_priority,
      group_order,
      forward,
      filter_type,
      start_location: Some(start_location),
      end_group: Some(end_group),
      subscribe_parameters,
    };

    let serialized = subscribe.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Subscribe as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = Subscribe::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let request_id = 128242;
    let track_namespace = Tuple::from_utf8_path("nein/nein/nein");
    let track_name = "${Name}".to_string();
    let subscriber_priority = 31;
    let group_order = GroupOrder::Original;
    let forward = true;
    let filter_type = FilterType::AbsoluteRange;
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let end_group = 25;
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"I'll sync you up")).unwrap(),
    ];
    let subscribe = Subscribe {
      request_id,
      track_namespace,
      track_name,
      subscriber_priority,
      group_order,
      forward,
      filter_type,
      start_location: Some(start_location),
      end_group: Some(end_group),
      subscribe_parameters,
    };

    let mut buf = subscribe.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Subscribe as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = Subscribe::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
