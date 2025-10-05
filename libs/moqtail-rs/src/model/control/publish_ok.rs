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
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct PublishOk {
  pub request_id: u64,
  pub forward: u8,
  pub subscriber_priority: u8,
  pub group_order: GroupOrder,
  pub filter_type: FilterType,
  pub start_location: Option<Location>,
  pub end_group: Option<u64>,
  pub parameters: Vec<KeyValuePair>,
}

#[allow(clippy::too_many_arguments)]
impl PublishOk {
  pub fn new(
    request_id: u64,
    forward: u8,
    subscriber_priority: u8,
    group_order: GroupOrder,
    filter_type: FilterType,
    start_location: Option<Location>,
    end_group: Option<u64>,
    parameters: Vec<KeyValuePair>,
  ) -> Self {
    Self {
      request_id,
      forward,
      subscriber_priority,
      group_order,
      filter_type,
      start_location,
      end_group,
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
    payload.put_u8(self.forward);
    payload.put_u8(self.subscriber_priority);
    payload.put_u8(self.group_order.into());
    payload.put_vi(self.filter_type as u64)?;

    // Conditional fields based on filter type
    match self.filter_type {
      FilterType::AbsoluteStart => {
        if let Some(ref location) = self.start_location {
          payload.extend_from_slice(&location.serialize()?);
        } else {
          return Err(ParseError::ProtocolViolation {
            context: "PublishOk::serialize",
            details: "AbsoluteStart filter requires start_location".to_string(),
          });
        }
      }
      FilterType::AbsoluteRange => {
        if let Some(ref location) = self.start_location {
          payload.extend_from_slice(&location.serialize()?);
        } else {
          return Err(ParseError::ProtocolViolation {
            context: "PublishOk::serialize",
            details: "AbsoluteRange filter requires start_location".to_string(),
          });
        }
        if let Some(end_group) = self.end_group {
          payload.put_vi(end_group)?;
        } else {
          return Err(ParseError::ProtocolViolation {
            context: "PublishOk::serialize",
            details: "AbsoluteRange filter requires end_group".to_string(),
          });
        }
      }
      _ => {
        // NextGroupStart and LatestObject don't require additional fields
      }
    }

    // Parameters
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
    let forward = payload.get_u8();
    if forward != 0 && forward != 1 {
      return Err(ParseError::ProtocolViolation {
        context: "PublishOk::parse_payload(forward)",
        details: format!("forward must be 0 or 1, got {}", forward),
      });
    }

    let subscriber_priority = payload.get_u8();
    let group_order = GroupOrder::try_from(payload.get_u8())?;
    let filter_type = FilterType::try_from(payload.get_vi()?)?;

    let (start_location, end_group) = match filter_type {
      FilterType::AbsoluteStart => {
        let location = Location::deserialize(payload)?;
        (Some(location), None)
      }
      FilterType::AbsoluteRange => {
        let location = Location::deserialize(payload)?;
        let end_group = payload.get_vi()?;
        (Some(location), Some(end_group))
      }
      _ => (None, None),
    };

    let param_count = payload.get_vi()? as usize;
    let mut parameters = Vec::with_capacity(param_count);
    for _ in 0..param_count {
      parameters.push(KeyValuePair::deserialize(payload)?);
    }

    Ok(Box::new(PublishOk {
      request_id,
      forward,
      subscriber_priority,
      group_order,
      filter_type,
      start_location,
      end_group,
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
  use bytes::Buf;

  #[test]
  fn test_roundtrip_latest_object() {
    let request_id = 123;
    let forward = 1;
    let subscriber_priority = 5;
    let group_order = GroupOrder::Ascending;
    let filter_type = FilterType::LatestObject;
    let start_location = None;
    let end_group = None;
    let parameters = vec![KeyValuePair::try_new_varint(0, 10).unwrap()];

    let publish_ok = PublishOk::new(
      request_id,
      forward,
      subscriber_priority,
      group_order,
      filter_type,
      start_location,
      end_group,
      parameters,
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
  fn test_roundtrip_absolute_range() {
    let request_id = 123;
    let forward = 0;
    let subscriber_priority = 3;
    let group_order = GroupOrder::Descending;
    let filter_type = FilterType::AbsoluteRange;
    let start_location = Some(Location::new(5, 10));
    let end_group = Some(100);
    let parameters = vec![];

    let publish_ok = PublishOk::new(
      request_id,
      forward,
      subscriber_priority,
      group_order,
      filter_type,
      start_location,
      end_group,
      parameters,
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
  fn test_invalid_absolute_start_without_location() {
    let request_id = 123;
    let forward = 1;
    let subscriber_priority = 5;
    let group_order = GroupOrder::Ascending;
    let filter_type = FilterType::AbsoluteStart;
    let start_location = None; // Should have location for AbsoluteStart
    let end_group = None;
    let parameters = vec![];

    let publish_ok = PublishOk::new(
      request_id,
      forward,
      subscriber_priority,
      group_order,
      filter_type,
      start_location,
      end_group,
      parameters,
    );

    let result = publish_ok.serialize();
    assert!(result.is_err());
  }
}
