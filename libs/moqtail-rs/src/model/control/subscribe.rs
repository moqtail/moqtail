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
use crate::model::common::tuple::{Tuple, TupleField};
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::data::full_track_name::FullTrackName;
use crate::model::error::ParseError;
use crate::model::parameter::message_parameter::{
  MessageParameter, deserialize_message_parameters,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct Subscribe {
  pub request_id: u64,
  pub track_namespace: Tuple,
  pub track_name: TupleField,
  pub subscribe_parameters: Vec<MessageParameter>,
}

impl Subscribe {
  fn build_base_params(
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
  ) -> Vec<MessageParameter> {
    let mut params = vec![
      MessageParameter::new_subscriber_priority(subscriber_priority),
      MessageParameter::new_forward(forward),
    ];
    if !matches!(group_order, GroupOrder::Original) {
      params.push(MessageParameter::new_group_order(group_order));
    }
    params
  }

  pub fn new_next_group_start(
    request_id: u64,
    track_namespace: Tuple,
    track_name: TupleField,
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
    subscribe_parameters: Vec<MessageParameter>,
  ) -> Self {
    let mut params = Self::build_base_params(subscriber_priority, group_order, forward);
    params.push(MessageParameter::new_subscription_filter(
      FilterType::NextGroupStart,
      None,
      None,
    ));
    params.extend(subscribe_parameters);
    Self {
      request_id,
      track_namespace,
      track_name,
      subscribe_parameters: params,
    }
  }

  pub fn new_latest_object(
    request_id: u64,
    track_namespace: Tuple,
    track_name: TupleField,
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
    subscribe_parameters: Vec<MessageParameter>,
  ) -> Self {
    let mut params = Self::build_base_params(subscriber_priority, group_order, forward);
    params.push(MessageParameter::new_subscription_filter(
      FilterType::LatestObject,
      None,
      None,
    ));
    params.extend(subscribe_parameters);
    Self {
      request_id,
      track_namespace,
      track_name,
      subscribe_parameters: params,
    }
  }

  pub fn new_absolute_start(
    request_id: u64,
    track_namespace: Tuple,
    track_name: TupleField,
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
    start_location: Location,
    subscribe_parameters: Vec<MessageParameter>,
  ) -> Self {
    let mut params = Self::build_base_params(subscriber_priority, group_order, forward);
    params.push(MessageParameter::new_subscription_filter(
      FilterType::AbsoluteStart,
      Some(start_location),
      None,
    ));
    params.extend(subscribe_parameters);
    Self {
      request_id,
      track_namespace,
      track_name,
      subscribe_parameters: params,
    }
  }

  pub fn new_absolute_range(
    request_id: u64,
    track_namespace: Tuple,
    track_name: TupleField,
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
    start_location: Location,
    end_group: u64,
    subscribe_parameters: Vec<MessageParameter>,
  ) -> Self {
    assert!(
      end_group >= start_location.group,
      "End Group must be >= Start Group"
    );
    let mut params = Self::build_base_params(subscriber_priority, group_order, forward);
    params.push(MessageParameter::new_subscription_filter(
      FilterType::AbsoluteRange,
      Some(start_location),
      Some(end_group),
    ));
    params.extend(subscribe_parameters);
    Self {
      request_id,
      track_namespace,
      track_name,
      subscribe_parameters: params,
    }
  }

  pub fn get_full_track_name(&self) -> FullTrackName {
    FullTrackName {
      namespace: self.track_namespace.clone(),
      name: self.track_name.clone(),
    }
  }

  /// Returns the SubscriptionFilter parameter if present.
  pub fn get_subscription_filter(&self) -> Option<(FilterType, Option<Location>, Option<u64>)> {
    self.subscribe_parameters.iter().find_map(|p| {
      if let MessageParameter::SubscriptionFilter {
        filter_type,
        start_location,
        end_group,
      } = p
      {
        Some((*filter_type, start_location.clone(), *end_group))
      } else {
        None
      }
    })
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
    let track_name = TupleField::new(payload.copy_to_bytes(name_len));

    let param_count = payload.get_vi()?;
    let subscribe_parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::Subscribe)?;

    Ok(Box::new(Subscribe {
      request_id,
      track_namespace,
      track_name,
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
  use crate::model::control::constant::GroupOrder;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let request_id = 128242;
    let track_namespace = Tuple::from_utf8_path("nein/nein/nein");
    let track_name = TupleField::from_utf8("${Name}");
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let subscribe = Subscribe::new_absolute_range(
      request_id,
      track_namespace,
      track_name,
      31,
      GroupOrder::Original,
      false,
      start_location,
      100,
      vec![],
    );

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
    let track_name = TupleField::from_utf8("${Name}");
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let subscribe = Subscribe::new_absolute_range(
      request_id,
      track_namespace,
      track_name,
      31,
      GroupOrder::Ascending,
      true,
      start_location,
      100,
      vec![],
    );

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
    let track_name = TupleField::from_utf8("${Name}");
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let subscribe = Subscribe::new_absolute_range(
      request_id,
      track_namespace,
      track_name,
      31,
      GroupOrder::Ascending,
      true,
      start_location,
      100,
      vec![],
    );

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
