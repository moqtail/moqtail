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

use crate::model::common::location::Location;
use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::control::constant::{ControlMessageType, FilterType, GroupOrder};
use crate::model::error::ParseError;
use crate::model::parameter::authorization_token::AuthorizationToken;
use crate::model::parameter::constant::MessageParameterType;
use bytes::{Bytes, BytesMut};

#[derive(Debug, Clone, PartialEq)]
pub enum MessageParameter {
  DeliveryTimeout {
    timeout: u64,
  },
  AuthorizationToken {
    token: AuthorizationToken,
  },
  Expires {
    expires: u64,
  },
  LargestObject {
    location: Location,
  },
  Forward {
    forward: bool,
  },
  SubscriberPriority {
    priority: u8,
  },
  GroupOrder {
    order: GroupOrder,
  },
  SubscriptionFilter {
    filter_type: FilterType,
    start_location: Option<Location>,
    end_group: Option<u64>,
  },
  NewGroupRequest {
    group: u64,
  },
}

impl MessageParameter {
  pub fn new_delivery_timeout(timeout: u64) -> Self {
    Self::DeliveryTimeout { timeout }
  }

  pub fn new_authorization_token(token: AuthorizationToken) -> Self {
    Self::AuthorizationToken { token }
  }

  pub fn new_expires(expires: u64) -> Self {
    Self::Expires { expires }
  }

  pub fn new_largest_object(location: Location) -> Self {
    Self::LargestObject { location }
  }

  pub fn new_forward(forward: bool) -> Self {
    Self::Forward { forward }
  }

  pub fn new_subscriber_priority(priority: u8) -> Self {
    Self::SubscriberPriority { priority }
  }

  pub fn new_group_order(order: GroupOrder) -> Self {
    Self::GroupOrder { order }
  }

  pub fn new_subscription_filter(
    filter_type: FilterType,
    start_location: Option<Location>,
    end_group: Option<u64>,
  ) -> Self {
    Self::SubscriptionFilter {
      filter_type,
      start_location,
      end_group,
    }
  }

  pub fn new_group_request(group: u64) -> Self {
    Self::NewGroupRequest { group }
  }

  /// Returns the raw wire type value for this parameter.
  pub fn type_value(&self) -> u64 {
    match self {
      Self::DeliveryTimeout { .. } => MessageParameterType::DeliveryTimeout as u64,
      Self::AuthorizationToken { .. } => MessageParameterType::AuthorizationToken as u64,
      Self::Expires { .. } => MessageParameterType::Expires as u64,
      Self::LargestObject { .. } => MessageParameterType::LargestObject as u64,
      Self::Forward { .. } => MessageParameterType::Forward as u64,
      Self::SubscriberPriority { .. } => MessageParameterType::SubscriberPriority as u64,
      Self::GroupOrder { .. } => MessageParameterType::GroupOrder as u64,
      Self::SubscriptionFilter { .. } => MessageParameterType::SubscriptionFilter as u64,
      Self::NewGroupRequest { .. } => MessageParameterType::NewGroupRequest as u64,
    }
  }

  /// Returns true if this parameter is permitted in the given control message type.
  /// Per spec: if a known parameter appears in a message where it is not defined,
  /// it MUST be ignored by the receiver.
  pub fn is_valid_for(&self, msg_type: ControlMessageType) -> bool {
    match self {
      Self::AuthorizationToken { .. } => matches!(
        msg_type,
        ControlMessageType::Publish
          | ControlMessageType::Subscribe
          | ControlMessageType::SubscribeUpdate
          | ControlMessageType::SubscribeNamespace
          | ControlMessageType::PublishNamespace
          | ControlMessageType::TrackStatus
          | ControlMessageType::Fetch
      ),
      Self::DeliveryTimeout { .. } => matches!(
        msg_type,
        ControlMessageType::PublishOk
          | ControlMessageType::Subscribe
          | ControlMessageType::SubscribeUpdate
      ),
      Self::SubscriberPriority { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::Fetch
          | ControlMessageType::SubscribeUpdate
          | ControlMessageType::PublishOk
      ),
      Self::GroupOrder { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::PublishOk
          | ControlMessageType::Fetch
          | ControlMessageType::SubscribeOk
          | ControlMessageType::FetchOk
          | ControlMessageType::Publish
      ),
      Self::SubscriptionFilter { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::PublishOk
          | ControlMessageType::SubscribeUpdate
      ),
      Self::Expires { .. } => matches!(
        msg_type,
        ControlMessageType::SubscribeOk
          | ControlMessageType::Publish
          | ControlMessageType::PublishOk
      ),
      Self::LargestObject { .. } => matches!(
        msg_type,
        ControlMessageType::SubscribeOk | ControlMessageType::Publish | ControlMessageType::FetchOk
      ),
      Self::Forward { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::SubscribeUpdate
          | ControlMessageType::Publish
          | ControlMessageType::PublishOk
          | ControlMessageType::SubscribeNamespace
      ),
      Self::NewGroupRequest { .. } => matches!(
        msg_type,
        ControlMessageType::PublishOk
          | ControlMessageType::Subscribe
          | ControlMessageType::SubscribeUpdate
      ),
    }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let kvp: KeyValuePair = self.clone().try_into()?;
    kvp.serialize()
  }

  /// Deserializes a single MessageParameter from a KeyValuePair.
  /// Returns ProtocolViolation for unrecognized parameter types.
  pub fn deserialize(kvp: &KeyValuePair) -> Result<Self, ParseError> {
    match kvp {
      KeyValuePair::VarInt { type_value, value } => {
        let param_type = MessageParameterType::try_from(*type_value).map_err(|_| {
          ParseError::ProtocolViolation {
            context: "MessageParameter::deserialize",
            details: format!("Unknown message parameter type: {type_value}"),
          }
        })?;
        match param_type {
          MessageParameterType::DeliveryTimeout => {
            if *value == 0 {
              return Err(ParseError::ProtocolViolation {
                context: "MessageParameter::deserialize",
                details: "DELIVERY_TIMEOUT must be greater than 0".to_string(),
              });
            }
            Ok(Self::DeliveryTimeout { timeout: *value })
          }
          MessageParameterType::Expires => Ok(Self::Expires { expires: *value }),
          MessageParameterType::Forward => match *value {
            0 => Ok(Self::Forward { forward: false }),
            1 => Ok(Self::Forward { forward: true }),
            _ => Err(ParseError::ProtocolViolation {
              context: "MessageParameter::deserialize",
              details: format!("FORWARD must be 0 or 1, got {value}"),
            }),
          },
          MessageParameterType::SubscriberPriority => {
            if *value > 255 {
              return Err(ParseError::ProtocolViolation {
                context: "MessageParameter::deserialize",
                details: format!("SUBSCRIBER_PRIORITY must be 0-255, got {value}"),
              });
            }
            Ok(Self::SubscriberPriority {
              priority: *value as u8,
            })
          }
          MessageParameterType::GroupOrder => match *value {
            0 => Ok(Self::GroupOrder {
              order: GroupOrder::Original,
            }),
            1 => Ok(Self::GroupOrder {
              order: GroupOrder::Ascending,
            }),
            2 => Ok(Self::GroupOrder {
              order: GroupOrder::Descending,
            }),
            _ => Err(ParseError::ProtocolViolation {
              context: "MessageParameter::deserialize",
              details: format!(
                "GROUP_ORDER must be 0 (Original), 1 (Ascending), or 2 (Descending), got {value}"
              ),
            }),
          },
          MessageParameterType::NewGroupRequest => Ok(Self::NewGroupRequest { group: *value }),
          _ => Err(ParseError::ProtocolViolation {
            context: "MessageParameter::deserialize",
            details: format!("Parameter type {type_value} is bytes-typed but received as varint"),
          }),
        }
      }
      KeyValuePair::Bytes { type_value, value } => {
        let param_type = MessageParameterType::try_from(*type_value).map_err(|_| {
          ParseError::ProtocolViolation {
            context: "MessageParameter::deserialize",
            details: format!("Unknown message parameter type: {type_value}"),
          }
        })?;
        match param_type {
          MessageParameterType::AuthorizationToken => {
            let mut payload = value.clone();
            let token = AuthorizationToken::deserialize(&mut payload)?;
            Ok(Self::AuthorizationToken { token })
          }
          MessageParameterType::LargestObject => {
            let mut payload = value.clone();
            let location = Location::deserialize(&mut payload)?;
            Ok(Self::LargestObject { location })
          }
          MessageParameterType::SubscriptionFilter => {
            let mut payload = value.clone();
            let ft_raw = payload.get_vi()?;
            let filter_type = FilterType::try_from(ft_raw)?;
            let (start_location, end_group) = match filter_type {
              FilterType::AbsoluteStart => {
                let loc = Location::deserialize(&mut payload)?;
                (Some(loc), None)
              }
              FilterType::AbsoluteRange => {
                let loc = Location::deserialize(&mut payload)?;
                let eg = payload.get_vi()?;
                (Some(loc), Some(eg))
              }
              _ => (None, None),
            };
            Ok(Self::SubscriptionFilter {
              filter_type,
              start_location,
              end_group,
            })
          }
          _ => Err(ParseError::ProtocolViolation {
            context: "MessageParameter::deserialize",
            details: format!("Parameter type {type_value} is varint-typed but received as bytes"),
          }),
        }
      }
    }
  }
}

/// Extension trait for `Vec<MessageParameter>` providing ergonomic get/set by type.
pub trait MessageParameterVecExt {
  /// Returns a reference to the first parameter matching the given type, if any.
  fn get_param(&self, param_type: MessageParameterType) -> Option<&MessageParameter>;
  /// Returns a clone of the first parameter matching the given type, or `default` if not found.
  fn get_param_or(
    &self,
    param_type: MessageParameterType,
    default: MessageParameter,
  ) -> MessageParameter;
  /// Inserts `param`, replacing any existing parameter of the same type.
  fn set_param(&mut self, param: MessageParameter);
}

impl MessageParameterVecExt for Vec<MessageParameter> {
  fn get_param(&self, param_type: MessageParameterType) -> Option<&MessageParameter> {
    self.iter().find(|p| p.type_value() == param_type as u64)
  }

  fn get_param_or(
    &self,
    param_type: MessageParameterType,
    default: MessageParameter,
  ) -> MessageParameter {
    self.get_param(param_type).cloned().unwrap_or(default)
  }

  fn set_param(&mut self, param: MessageParameter) {
    let type_value = param.type_value();
    if let Some(existing) = self.iter_mut().find(|p| p.type_value() == type_value) {
      *existing = param;
    } else {
      self.push(param);
    }
  }
}

impl TryInto<KeyValuePair> for MessageParameter {
  type Error = ParseError;

  fn try_into(self) -> Result<KeyValuePair, Self::Error> {
    match self {
      Self::DeliveryTimeout { timeout } => {
        KeyValuePair::try_new_varint(MessageParameterType::DeliveryTimeout as u64, timeout)
      }
      Self::Expires { expires } => {
        KeyValuePair::try_new_varint(MessageParameterType::Expires as u64, expires)
      }
      Self::Forward { forward } => KeyValuePair::try_new_varint(
        MessageParameterType::Forward as u64,
        if forward { 1 } else { 0 },
      ),
      Self::SubscriberPriority { priority } => KeyValuePair::try_new_varint(
        MessageParameterType::SubscriberPriority as u64,
        priority as u64,
      ),
      Self::GroupOrder { order } => {
        KeyValuePair::try_new_varint(MessageParameterType::GroupOrder as u64, order as u64)
      }
      Self::NewGroupRequest { group } => {
        KeyValuePair::try_new_varint(MessageParameterType::NewGroupRequest as u64, group)
      }
      Self::AuthorizationToken { token } => {
        let payload = token.serialize()?;
        KeyValuePair::try_new_bytes(MessageParameterType::AuthorizationToken as u64, payload)
      }
      Self::LargestObject { location } => {
        let mut buf = BytesMut::new();
        buf.put_vi(location.group)?;
        buf.put_vi(location.object)?;
        KeyValuePair::try_new_bytes(MessageParameterType::LargestObject as u64, buf.freeze())
      }
      Self::SubscriptionFilter {
        filter_type,
        start_location,
        end_group,
      } => {
        let mut buf = BytesMut::new();
        buf.put_vi(filter_type as u64)?;
        if matches!(
          filter_type,
          FilterType::AbsoluteStart | FilterType::AbsoluteRange
        ) && let Some(loc) = start_location
        {
          buf.put_vi(loc.group)?;
          buf.put_vi(loc.object)?;
        }
        if filter_type == FilterType::AbsoluteRange
          && let Some(eg) = end_group
        {
          buf.put_vi(eg)?;
        }
        KeyValuePair::try_new_bytes(
          MessageParameterType::SubscriptionFilter as u64,
          buf.freeze(),
        )
      }
    }
  }
}

/// Deserializes `count` MessageParameters from a raw byte buffer for the given message type.
/// - Unknown parameter types → ProtocolViolation error
/// - Known parameters not valid for `msg_type` → silently ignored per spec
pub fn deserialize_message_parameters(
  bytes: &mut Bytes,
  count: u64,
  msg_type: ControlMessageType,
) -> Result<Vec<MessageParameter>, ParseError> {
  let mut params = Vec::with_capacity(count as usize);
  for _ in 0..count {
    let kvp = KeyValuePair::deserialize(bytes)?;
    let param = MessageParameter::deserialize(&kvp)?;
    if param.is_valid_for(msg_type) {
      params.push(param);
    }
    // else: silently ignore per spec
  }
  Ok(params)
}

/// Serializes a Vec<MessageParameter> into a Vec<KeyValuePair>.
pub fn serialize_message_parameters(
  params: Vec<MessageParameter>,
) -> Result<Vec<KeyValuePair>, ParseError> {
  params.into_iter().map(|p| p.try_into()).collect()
}

/// Applies a set of parameter updates to an existing parameter list.
/// For each update, replaces the matching parameter (by type value) or appends it.
/// Per spec: "If omitted from REQUEST_UPDATE/SUBSCRIBE_UPDATE, the value is unchanged."
pub fn apply_message_parameter_update(
  current: &mut Vec<MessageParameter>,
  updates: Vec<MessageParameter>,
) {
  for update in updates {
    let update_type = update.type_value();
    if let Some(existing) = current.iter_mut().find(|p| p.type_value() == update_type) {
      *existing = update;
    } else {
      current.push(update);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::pair::KeyValuePair;
  use crate::model::parameter::authorization_token::AuthorizationToken;
  use bytes::{Buf, BytesMut};

  fn roundtrip(param: MessageParameter) -> MessageParameter {
    let serialized = param.serialize().unwrap();
    let mut bytes = serialized;
    let kvp = KeyValuePair::deserialize(&mut bytes).unwrap();
    let result = MessageParameter::deserialize(&kvp).unwrap();
    assert_eq!(bytes.remaining(), 0);
    result
  }

  #[test]
  fn test_roundtrip_delivery_timeout() {
    let orig = MessageParameter::new_delivery_timeout(0xABCD);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_expires() {
    let orig = MessageParameter::new_expires(9999);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_forward() {
    let orig = MessageParameter::new_forward(false);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_subscriber_priority() {
    let orig = MessageParameter::new_subscriber_priority(42);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_group_order() {
    let orig = MessageParameter::new_group_order(GroupOrder::Ascending);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_new_group_request() {
    let orig = MessageParameter::new_group_request(7);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_authorization_token() {
    let token = AuthorizationToken::new_use_alias(42);
    let orig = MessageParameter::new_authorization_token(token);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_largest_object() {
    let orig = MessageParameter::new_largest_object(Location {
      group: 10,
      object: 5,
    });
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_subscription_filter_latest_object() {
    let orig = MessageParameter::new_subscription_filter(FilterType::LatestObject, None, None);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_subscription_filter_absolute_start() {
    let orig = MessageParameter::new_subscription_filter(
      FilterType::AbsoluteStart,
      Some(Location {
        group: 3,
        object: 1,
      }),
      None,
    );
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_roundtrip_subscription_filter_absolute_range() {
    let orig = MessageParameter::new_subscription_filter(
      FilterType::AbsoluteRange,
      Some(Location {
        group: 5,
        object: 0,
      }),
      Some(20),
    );
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_unknown_type_is_protocol_violation() {
    let kvp = KeyValuePair::try_new_varint(998, 1).unwrap();
    let err = MessageParameter::deserialize(&kvp).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn test_bulk_deserialize_ignores_wrong_message_params() {
    // DeliveryTimeout is not valid in Fetch messages — should be silently dropped
    let params = vec![
      MessageParameter::new_delivery_timeout(100),
      MessageParameter::new_subscriber_priority(50),
    ];
    let kvps = serialize_message_parameters(params).unwrap();
    let mut buf = BytesMut::new();
    for kvp in &kvps {
      buf.extend_from_slice(&kvp.serialize().unwrap());
    }
    let mut bytes = buf.freeze();
    // DeliveryTimeout is invalid for Fetch; SubscriberPriority is valid
    let result =
      deserialize_message_parameters(&mut bytes, kvps.len() as u64, ControlMessageType::Fetch)
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], MessageParameter::new_subscriber_priority(50));
  }

  #[test]
  fn test_bulk_deserialize_errors_on_unknown_type() {
    let kvp = KeyValuePair::try_new_varint(998, 1).unwrap();
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&kvp.serialize().unwrap());
    let mut bytes = buf.freeze();
    let err =
      deserialize_message_parameters(&mut bytes, 1, ControlMessageType::Subscribe).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn test_apply_update() {
    let mut current = vec![
      MessageParameter::new_subscriber_priority(100),
      MessageParameter::new_forward(true),
    ];
    let updates = vec![
      MessageParameter::new_subscriber_priority(50),
      MessageParameter::new_delivery_timeout(500),
    ];
    apply_message_parameter_update(&mut current, updates);
    assert_eq!(current.len(), 3);
    assert!(current.contains(&MessageParameter::new_subscriber_priority(50)));
    assert!(current.contains(&MessageParameter::new_forward(true)));
    assert!(current.contains(&MessageParameter::new_delivery_timeout(500)));
  }

  #[test]
  fn test_is_valid_for() {
    let timeout = MessageParameter::new_delivery_timeout(100);
    assert!(timeout.is_valid_for(ControlMessageType::Subscribe));
    assert!(timeout.is_valid_for(ControlMessageType::PublishOk));
    assert!(!timeout.is_valid_for(ControlMessageType::Fetch));
  }

  #[test]
  fn test_type_value() {
    assert_eq!(
      MessageParameter::new_delivery_timeout(0).type_value(),
      MessageParameterType::DeliveryTimeout as u64
    );
    assert_eq!(
      MessageParameter::new_forward(true).type_value(),
      MessageParameterType::Forward as u64
    );
  }
}
