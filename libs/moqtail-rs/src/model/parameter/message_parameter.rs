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
use crate::model::common::tuple::Tuple;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::control::constant::{ControlMessageType, FilterType, GroupOrder, SwitchMode};
use crate::model::error::ParseError;
use crate::model::parameter::authorization_token::AuthorizationToken;
use crate::model::parameter::constant::MessageParameterType;
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, Clone, PartialEq)]
pub enum MessageParameter {
  ObjectDeliveryTimeout {
    timeout: u64,
  },
  SubgroupDeliveryTimeout {
    timeout: u64,
  },
  RendezvousTimeout {
    timeout: u64,
  },
  FillTimeout {
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
    /// How many groups back from the Largest Object a RelativeStartFill starts.
    relative_previous: Option<u64>,
  },
  SwitchFrom {
    /// The subscription to suspend.
    request_id: u64,
    mode: SwitchMode,
    /// Whether the publisher sends PUBLISH_DONE on the suspended subscription.
    publish_done: bool,
  },
  /// Overrides for the fill fetch stream. An omitted parameter keeps the value it
  /// has for the live subscription. Ignored without a fill filter type.
  FillParameters {
    parameters: Vec<MessageParameter>,
  },
  NewGroupRequest {
    group: u64,
  },
  TrackNamespacePrefix {
    prefix: Tuple,
  },
}

/// Publish Done is the top bit of the flags byte; the rest are reserved.
const SWITCH_FROM_PUBLISH_DONE: u8 = 0b1000_0000;

impl MessageParameter {
  pub fn new_object_delivery_timeout(timeout: u64) -> Self {
    Self::ObjectDeliveryTimeout { timeout }
  }

  pub fn new_subgroup_delivery_timeout(timeout: u64) -> Self {
    Self::SubgroupDeliveryTimeout { timeout }
  }

  pub fn new_rendezvous_timeout(timeout: u64) -> Self {
    Self::RendezvousTimeout { timeout }
  }

  pub fn new_fill_timeout(timeout: u64) -> Self {
    Self::FillTimeout { timeout }
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
      relative_previous: None,
    }
  }

  pub fn new_relative_start_fill(relative_previous: u64) -> Self {
    Self::SubscriptionFilter {
      filter_type: FilterType::RelativeStartFill,
      start_location: None,
      end_group: None,
      relative_previous: Some(relative_previous),
    }
  }

  pub fn new_fill_parameters(parameters: Vec<MessageParameter>) -> Self {
    Self::FillParameters { parameters }
  }

  pub fn new_switch_from(request_id: u64, mode: SwitchMode, publish_done: bool) -> Self {
    Self::SwitchFrom {
      request_id,
      mode,
      publish_done,
    }
  }

  pub fn new_track_namespace_prefix(prefix: Tuple) -> Self {
    Self::TrackNamespacePrefix { prefix }
  }

  pub fn new_group_request(group: u64) -> Self {
    Self::NewGroupRequest { group }
  }

  /// Returns the raw wire type value for this parameter.
  pub fn type_value(&self) -> u64 {
    match self {
      Self::ObjectDeliveryTimeout { .. } => MessageParameterType::ObjectDeliveryTimeout as u64,
      Self::SubgroupDeliveryTimeout { .. } => MessageParameterType::SubgroupDeliveryTimeout as u64,
      Self::RendezvousTimeout { .. } => MessageParameterType::RendezvousTimeout as u64,
      Self::FillTimeout { .. } => MessageParameterType::FillTimeout as u64,
      Self::AuthorizationToken { .. } => MessageParameterType::AuthorizationToken as u64,
      Self::Expires { .. } => MessageParameterType::Expires as u64,
      Self::LargestObject { .. } => MessageParameterType::LargestObject as u64,
      Self::Forward { .. } => MessageParameterType::Forward as u64,
      Self::SubscriberPriority { .. } => MessageParameterType::SubscriberPriority as u64,
      Self::GroupOrder { .. } => MessageParameterType::GroupOrder as u64,
      Self::SubscriptionFilter { .. } => MessageParameterType::SubscriptionFilter as u64,
      Self::FillParameters { .. } => MessageParameterType::FillParameters as u64,
      Self::SwitchFrom { .. } => MessageParameterType::SwitchFrom as u64,
      Self::NewGroupRequest { .. } => MessageParameterType::NewGroupRequest as u64,
      Self::TrackNamespacePrefix { .. } => MessageParameterType::TrackNamespacePrefix as u64,
    }
  }

  /// Returns true if this parameter is permitted in the given control message type.
  /// A parameter appearing in a message type it is not defined for is a
  /// PROTOCOL_VIOLATION.
  pub fn is_valid_for(&self, msg_type: ControlMessageType) -> bool {
    match self {
      Self::AuthorizationToken { .. } => matches!(
        msg_type,
        ControlMessageType::Publish
          | ControlMessageType::Subscribe
          | ControlMessageType::RequestUpdate
          | ControlMessageType::SubscribeNamespace
          | ControlMessageType::SubscribeTracks
          | ControlMessageType::PublishNamespace
          | ControlMessageType::TrackStatus
          | ControlMessageType::Fetch
      ),
      // PUBLISH_OK, SUBSCRIBE, or REQUEST_UPDATE.
      Self::ObjectDeliveryTimeout { .. } | Self::SubgroupDeliveryTimeout { .. } => matches!(
        msg_type,
        ControlMessageType::PublishOk
          | ControlMessageType::RequestOk
          | ControlMessageType::Subscribe
          | ControlMessageType::RequestUpdate
      ),
      // SUBSCRIBE only.
      Self::RendezvousTimeout { .. } => matches!(msg_type, ControlMessageType::Subscribe),
      // FETCH only.
      Self::FillTimeout { .. } => matches!(msg_type, ControlMessageType::Fetch),
      Self::SubscriberPriority { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::Fetch
          | ControlMessageType::RequestUpdate
          | ControlMessageType::PublishOk
          | ControlMessageType::RequestOk
      ),
      Self::GroupOrder { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::PublishOk
          | ControlMessageType::Fetch
          | ControlMessageType::SubscribeOk
          | ControlMessageType::FetchOk
          | ControlMessageType::Publish
          | ControlMessageType::RequestOk
      ),
      Self::SubscriptionFilter { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::PublishOk
          | ControlMessageType::RequestOk
          | ControlMessageType::RequestUpdate
      ),
      Self::SwitchFrom { .. } | Self::FillParameters { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe | ControlMessageType::RequestUpdate
      ),
      Self::Expires { .. } => matches!(
        msg_type,
        ControlMessageType::SubscribeOk
          | ControlMessageType::Publish
          | ControlMessageType::PublishOk
          | ControlMessageType::RequestOk
      ),
      Self::LargestObject { .. } => matches!(
        msg_type,
        ControlMessageType::SubscribeOk
          | ControlMessageType::Publish
          | ControlMessageType::FetchOk
          | ControlMessageType::RequestOk
      ),
      Self::Forward { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::RequestUpdate
          | ControlMessageType::Publish
          | ControlMessageType::PublishOk
          | ControlMessageType::RequestOk
          | ControlMessageType::SubscribeNamespace
          | ControlMessageType::SubscribeTracks
      ),
      Self::NewGroupRequest { .. } => matches!(
        msg_type,
        ControlMessageType::PublishOk
          | ControlMessageType::RequestOk
          | ControlMessageType::Subscribe
          | ControlMessageType::RequestUpdate
      ),
      Self::TrackNamespacePrefix { .. } => matches!(msg_type, ControlMessageType::RequestUpdate),
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
          // A value of 0 means no timeout is set. It is valid, not a violation.
          MessageParameterType::ObjectDeliveryTimeout => {
            Ok(Self::ObjectDeliveryTimeout { timeout: *value })
          }
          MessageParameterType::SubgroupDeliveryTimeout => {
            Ok(Self::SubgroupDeliveryTimeout { timeout: *value })
          }
          MessageParameterType::RendezvousTimeout => {
            Ok(Self::RendezvousTimeout { timeout: *value })
          }
          MessageParameterType::FillTimeout => Ok(Self::FillTimeout { timeout: *value }),
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
          MessageParameterType::TrackNamespacePrefix => {
            let mut payload = value.clone();
            let prefix = Tuple::deserialize(&mut payload)?;
            if payload.has_remaining() {
              return Err(ParseError::KeyValueFormattingError {
                context: "MessageParameter::deserialize(TrackNamespacePrefix)",
              });
            }
            Ok(Self::TrackNamespacePrefix { prefix })
          }
          MessageParameterType::FillParameters => {
            let mut payload = value.clone();
            let mut parameters = Vec::new();
            let mut prev_type = 0u64;
            while payload.has_remaining() {
              let delta_type = payload.get_vi()?;
              let type_value =
                prev_type
                  .checked_add(delta_type)
                  .ok_or_else(|| ParseError::ProtocolViolation {
                    context: "MessageParameter::deserialize(FillParameters)",
                    details: format!(
                      "previous type {prev_type} plus delta type {delta_type} exceeds 2^64 - 1"
                    ),
                  })?;
              prev_type = type_value;
              if !is_valid_fill_parameter(type_value) {
                return Err(ParseError::ProtocolViolation {
                  context: "MessageParameter::deserialize(FillParameters)",
                  details: format!(
                    "parameter type 0x{type_value:02X} is not allowed inside FILL_PARAMETERS"
                  ),
                });
              }
              let kvp = read_message_parameter_value(&mut payload, type_value)?;
              parameters.push(MessageParameter::deserialize(&kvp)?);
            }
            Ok(Self::FillParameters { parameters })
          }
          MessageParameterType::SwitchFrom => {
            let mut payload = value.clone();
            let request_id = payload.get_vi()?;
            let mode = SwitchMode::try_from(payload.get_vi()?)?;
            if !payload.has_remaining() {
              return Err(ParseError::NotEnoughBytes {
                context: "MessageParameter::deserialize(SwitchFrom flags)",
                needed: 1,
                available: 0,
              });
            }
            let flags = payload.get_u8();
            if flags & !SWITCH_FROM_PUBLISH_DONE != 0 {
              return Err(ParseError::ProtocolViolation {
                context: "MessageParameter::deserialize",
                details: format!("SWITCH_FROM reserved bits must be 0, got {flags:#04x}"),
              });
            }
            if payload.has_remaining() {
              return Err(ParseError::KeyValueFormattingError {
                context: "MessageParameter::deserialize(SwitchFrom)",
              });
            }
            Ok(Self::SwitchFrom {
              request_id,
              mode,
              publish_done: flags & SWITCH_FROM_PUBLISH_DONE != 0,
            })
          }
          MessageParameterType::SubscriptionFilter => {
            let mut payload = value.clone();
            let ft_raw = payload.get_vi()?;
            let filter_type = FilterType::try_from(ft_raw)?;
            let (start_location, end_group, relative_previous) = match filter_type {
              FilterType::AbsoluteStartFill => {
                let loc = Location::deserialize(&mut payload)?;
                (Some(loc), None, None)
              }
              FilterType::AbsoluteRangeFill => {
                let loc = Location::deserialize(&mut payload)?;
                // End Group is a delta from the Start Group on the wire.
                let delta = payload.get_vi()?;
                let end_group =
                  loc
                    .group
                    .checked_add(delta)
                    .ok_or_else(|| ParseError::ProtocolViolation {
                      context: "MessageParameter::deserialize",
                      details: "AbsoluteRangeFill End Group Delta overflows u64".to_string(),
                    })?;
                (Some(loc), Some(end_group), None)
              }
              FilterType::RelativeStartFill => (None, None, Some(payload.get_vi()?)),
              _ => (None, None, None),
            };
            Ok(Self::SubscriptionFilter {
              filter_type,
              start_location,
              end_group,
              relative_previous,
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
      Self::ObjectDeliveryTimeout { timeout } => {
        KeyValuePair::try_new_varint(MessageParameterType::ObjectDeliveryTimeout as u64, timeout)
      }
      Self::SubgroupDeliveryTimeout { timeout } => KeyValuePair::try_new_varint(
        MessageParameterType::SubgroupDeliveryTimeout as u64,
        timeout,
      ),
      Self::RendezvousTimeout { timeout } => {
        KeyValuePair::try_new_varint(MessageParameterType::RendezvousTimeout as u64, timeout)
      }
      Self::FillTimeout { timeout } => {
        KeyValuePair::try_new_varint(MessageParameterType::FillTimeout as u64, timeout)
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
      Self::FillParameters { parameters } => {
        for param in &parameters {
          if !is_valid_fill_parameter(param.type_value()) {
            return Err(ParseError::ProtocolViolation {
              context: "MessageParameter::try_into(FillParameters)",
              details: format!(
                "parameter type 0x{:02X} is not allowed inside FILL_PARAMETERS",
                param.type_value()
              ),
            });
          }
        }
        KeyValuePair::try_new_bytes(
          MessageParameterType::FillParameters as u64,
          serialize_message_parameters(&parameters)?,
        )
      }
      Self::SwitchFrom {
        request_id,
        mode,
        publish_done,
      } => {
        let mut buf = BytesMut::new();
        buf.put_vi(request_id)?;
        buf.put_vi(mode as u64)?;
        buf.put_u8(if publish_done {
          SWITCH_FROM_PUBLISH_DONE
        } else {
          0
        });
        KeyValuePair::new_bytes(MessageParameterType::SwitchFrom as u64, buf.freeze())
      }
      Self::SubscriptionFilter {
        filter_type,
        start_location,
        end_group,
        relative_previous,
      } => {
        let mut buf = BytesMut::new();
        buf.put_vi(filter_type as u64)?;
        let start_group = start_location.as_ref().map(|l| l.group).unwrap_or(0);
        if matches!(
          filter_type,
          FilterType::AbsoluteStartFill | FilterType::AbsoluteRangeFill
        ) && let Some(loc) = &start_location
        {
          buf.put_vi(loc.group)?;
          buf.put_vi(loc.object)?;
        }
        if filter_type == FilterType::AbsoluteRangeFill
          && let Some(eg) = end_group
        {
          // End Group is encoded on the wire as a delta from the Start Group.
          buf.put_vi(eg.saturating_sub(start_group))?;
        }
        if filter_type == FilterType::RelativeStartFill {
          buf.put_vi(relative_previous.unwrap_or(0))?;
        }
        KeyValuePair::try_new_bytes(
          MessageParameterType::SubscriptionFilter as u64,
          buf.freeze(),
        )
      }
      // A namespace tuple: length-prefixed, even though the Type is even.
      Self::TrackNamespacePrefix { prefix } => KeyValuePair::new_bytes(
        MessageParameterType::TrackNamespacePrefix as u64,
        prefix.serialize()?,
      ),
    }
  }
}

/// Deserializes `count` MessageParameters from a raw byte buffer for the given message type.
/// - Unknown parameter types → ProtocolViolation error
/// - Known parameters not valid for `msg_type` → ProtocolViolation error
///
/// A parameter appearing in a message type it is not defined for MUST close the
/// session with PROTOCOL_VIOLATION.
pub fn deserialize_message_parameters(
  bytes: &mut Bytes,
  count: u64,
  msg_type: ControlMessageType,
) -> Result<Vec<MessageParameter>, ParseError> {
  let mut params = Vec::with_capacity(count as usize);
  let mut prev_type = 0u64;
  for _ in 0..count {
    let delta_type = bytes.get_vi()?;
    let type_value =
      prev_type
        .checked_add(delta_type)
        .ok_or_else(|| ParseError::ProtocolViolation {
          context: "deserialize_message_parameters",
          details: format!(
            "previous type {prev_type} plus delta type {delta_type} exceeds 2^64 - 1"
          ),
        })?;
    prev_type = type_value;

    let kvp = read_message_parameter_value(bytes, type_value)?;

    let param = MessageParameter::deserialize(&kvp)?;
    if !param.is_valid_for(msg_type) {
      return Err(ParseError::ProtocolViolation {
        context: "deserialize_message_parameters",
        details: format!(
          "parameter type 0x{:02X} is not allowed in {msg_type:?}",
          param.type_value()
        ),
      });
    }
    params.push(param);
  }
  Ok(params)
}

/// How a message parameter's Value is laid out on the wire. The parameter Type
/// decides this, not its parity, and reading one the wrong way desyncs the rest
/// of the list. The same Type number means something else in the setup-option
/// namespace, so this belongs here and not in the shared codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueShape {
  VarInt,
  /// A single byte: FORWARD, SUBSCRIBER_PRIORITY, GROUP_ORDER.
  Uint8,
  /// Two varints, no length prefix: LARGEST_OBJECT.
  BareLocation,
  LengthPrefixedBytes,
}

/// The only parameters that may override anything for the fill fetch stream.
const fn is_valid_fill_parameter(type_value: u64) -> bool {
  type_value == MessageParameterType::FillTimeout as u64
    || type_value == MessageParameterType::SubscriberPriority as u64
    || type_value == MessageParameterType::GroupOrder as u64
}

/// Reads one parameter Value whose Type has already been decoded.
fn read_message_parameter_value(
  bytes: &mut Bytes,
  type_value: u64,
) -> Result<KeyValuePair, ParseError> {
  match value_shape(type_value) {
    ValueShape::Uint8 => {
      if !bytes.has_remaining() {
        return Err(ParseError::NotEnoughBytes {
          context: "read_message_parameter_value(uint8)",
          needed: 1,
          available: 0,
        });
      }
      Ok(KeyValuePair::VarInt {
        type_value,
        value: bytes.get_u8() as u64,
      })
    }
    ValueShape::BareLocation => {
      let mut loc = BytesMut::new();
      loc.put_vi(bytes.get_vi()?)?;
      loc.put_vi(bytes.get_vi()?)?;
      KeyValuePair::new_bytes(type_value, loc.freeze())
    }
    ValueShape::LengthPrefixedBytes => KeyValuePair::deserialize_bytes_value(bytes, type_value),
    ValueShape::VarInt => Ok(KeyValuePair::VarInt {
      type_value,
      value: bytes.get_vi()?,
    }),
  }
}

const fn value_shape(type_value: u64) -> ValueShape {
  if type_value == MessageParameterType::Forward as u64
    || type_value == MessageParameterType::SubscriberPriority as u64
    || type_value == MessageParameterType::GroupOrder as u64
  {
    ValueShape::Uint8
  } else if type_value == MessageParameterType::LargestObject as u64 {
    ValueShape::BareLocation
  } else if type_value == MessageParameterType::TrackNamespacePrefix as u64
    || type_value == MessageParameterType::SwitchFrom as u64
  {
    ValueShape::LengthPrefixedBytes
  } else if type_value.is_multiple_of(2) {
    ValueShape::VarInt
  } else {
    ValueShape::LengthPrefixedBytes
  }
}

/// Serializes a slice of MessageParameters into delta-encoded wire bytes,
/// ready to be appended directly to a message payload. Parameters are sorted by
/// ascending Type first, since the Type Delta is an unsigned varint and cannot
/// represent a decrease.
pub fn serialize_message_parameters(params: &[MessageParameter]) -> Result<Bytes, ParseError> {
  let mut kvps: Vec<KeyValuePair> = params
    .iter()
    .map(|p| p.clone().try_into())
    .collect::<Result<_, ParseError>>()?;
  kvps.sort_by_key(|kvp| kvp.get_type());

  let mut buf = BytesMut::new();
  let mut prev_type = 0u64;
  for kvp in &kvps {
    let type_value = kvp.get_type();
    let delta_type =
      type_value
        .checked_sub(prev_type)
        .ok_or_else(|| ParseError::ProtocolViolation {
          context: "serialize_message_parameters",
          details: format!("type {type_value} is less than previous type {prev_type}"),
        })?;
    buf.put_vi(delta_type)?;
    match (value_shape(type_value), kvp) {
      (ValueShape::Uint8, KeyValuePair::VarInt { value, .. }) => {
        let byte: u8 = (*value)
          .try_into()
          .map_err(|_| ParseError::ProtocolViolation {
            context: "serialize_message_parameters",
            details: format!("uint8 parameter 0x{type_value:02X} value {value} exceeds 255"),
          })?;
        buf.put_u8(byte);
      }
      (ValueShape::BareLocation, KeyValuePair::Bytes { value, .. }) => {
        buf.extend_from_slice(value);
      }
      (_, KeyValuePair::VarInt { value, .. }) => buf.put_vi(*value)?,
      (_, KeyValuePair::Bytes { value, .. }) => {
        buf.put_vi(value.len() as u64)?;
        buf.extend_from_slice(value);
      }
    }
    prev_type = type_value;
  }
  Ok(buf.freeze())
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
  use crate::model::common::tuple::TupleField;

  /// LARGEST_OBJECT is a bare Location: Type Delta 0x09 followed by the Group
  /// and Object varints, with no length prefix. These are the bytes another
  /// implementation put on the wire in a SUBSCRIBE_OK carrying {1, 13}.
  #[test]
  fn largest_object_is_an_unprefixed_location() {
    let wire = Bytes::from_static(&[0x09, 0x01, 0x0d]);

    let mut buf = wire.clone();
    let params =
      deserialize_message_parameters(&mut buf, 1, ControlMessageType::SubscribeOk).unwrap();

    assert_eq!(
      params,
      vec![MessageParameter::LargestObject {
        location: Location {
          group: 1,
          object: 13
        }
      }]
    );
    assert!(!buf.has_remaining(), "the parameter must consume 3 bytes");
    assert_eq!(serialize_message_parameters(&params).unwrap(), wire);
  }
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
    let orig = MessageParameter::new_object_delivery_timeout(0xABCD);
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
      FilterType::AbsoluteStartFill,
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
      FilterType::AbsoluteRangeFill,
      Some(Location {
        group: 5,
        object: 0,
      }),
      Some(20),
    );
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_absolute_range_end_group_is_delta_on_wire() {
    // Start group 5, absolute End Group 20 must serialize End Group as the
    // delta 15, not the absolute 20.
    let orig = MessageParameter::new_subscription_filter(
      FilterType::AbsoluteRangeFill,
      Some(Location {
        group: 5,
        object: 0,
      }),
      Some(20),
    );
    let mut bytes = orig.serialize().unwrap();
    let kvp = KeyValuePair::deserialize(&mut bytes).unwrap();
    let KeyValuePair::Bytes { value, .. } = kvp else {
      panic!("SubscriptionFilter must be a bytes KVP");
    };
    let mut value = value;
    assert_eq!(
      value.get_vi().unwrap(),
      FilterType::AbsoluteRangeFill as u64
    );
    assert_eq!(value.get_vi().unwrap(), 5); // start group
    assert_eq!(value.get_vi().unwrap(), 0); // start object
    assert_eq!(value.get_vi().unwrap(), 15); // End Group Delta = 20 - 5
  }

  #[test]
  fn test_absolute_range_end_group_delta_overflow_is_protocol_violation() {
    // Start group u64::MAX plus a non-zero delta overflows the absolute Group ID
    // and MUST be rejected.
    let mut value = BytesMut::new();
    value.put_vi(FilterType::AbsoluteRangeFill as u64).unwrap();
    value.put_vi(u64::MAX).unwrap(); // start group
    value.put_vi(0u64).unwrap(); // start object
    value.put_vi(1u64).unwrap(); // End Group Delta -> overflow
    let kvp = KeyValuePair::try_new_bytes(
      MessageParameterType::SubscriptionFilter as u64,
      value.freeze(),
    )
    .unwrap();
    assert!(matches!(
      MessageParameter::deserialize(&kvp),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn test_roundtrip_subscription_filter_relative_start_fill() {
    let orig = MessageParameter::new_relative_start_fill(3);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  #[test]
  fn test_relative_start_fill_carries_relative_previous_on_the_wire() {
    let mut bytes = MessageParameter::new_relative_start_fill(3)
      .serialize()
      .unwrap();
    let kvp = KeyValuePair::deserialize(&mut bytes).unwrap();
    let KeyValuePair::Bytes { value, .. } = kvp else {
      panic!("SubscriptionFilter must be a bytes KVP");
    };
    let mut value = value;
    assert_eq!(
      value.get_vi().unwrap(),
      FilterType::RelativeStartFill as u64
    );
    assert_eq!(value.get_vi().unwrap(), 3);
    assert!(!value.has_remaining());
  }

  #[test]
  fn test_roundtrip_fill_parameters() {
    let params = vec![MessageParameter::new_fill_parameters(vec![
      MessageParameter::new_fill_timeout(3000),
      MessageParameter::new_subscriber_priority(10),
      MessageParameter::new_group_order(GroupOrder::Descending),
    ])];
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let decoded =
      deserialize_message_parameters(&mut bytes, 1, ControlMessageType::Subscribe).unwrap();
    assert_eq!(decoded, params);
    assert!(!bytes.has_remaining());
  }

  #[test]
  fn test_fill_parameters_wire_format() {
    let wire = Bytes::from_static(&[0x23, 0x03, 0x0A, 0x8B, 0xB8]);
    let params = vec![MessageParameter::new_fill_parameters(vec![
      MessageParameter::new_fill_timeout(3000),
    ])];

    assert_eq!(serialize_message_parameters(&params).unwrap(), wire);

    let mut buf = wire.clone();
    let decoded =
      deserialize_message_parameters(&mut buf, 1, ControlMessageType::Subscribe).unwrap();
    assert_eq!(decoded, params);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_fill_parameters_rejects_a_parameter_it_cannot_override() {
    // FORWARD belongs to the subscription, not to its fill.
    let params = vec![MessageParameter::new_fill_parameters(vec![
      MessageParameter::new_forward(true),
    ])];
    assert!(matches!(
      serialize_message_parameters(&params),
      Err(ParseError::ProtocolViolation { .. })
    ));

    // And the same on the way in, where a peer sent it anyway.
    let inner = {
      let mut buf = BytesMut::new();
      buf.put_vi(MessageParameterType::Forward as u64).unwrap();
      buf.put_u8(1);
      buf.freeze()
    };
    let kvp =
      KeyValuePair::try_new_bytes(MessageParameterType::FillParameters as u64, inner).unwrap();
    assert!(matches!(
      MessageParameter::deserialize(&kvp),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn test_fill_parameters_rejected_outside_subscribe_and_request_update() {
    let params = vec![MessageParameter::new_fill_parameters(vec![
      MessageParameter::new_fill_timeout(1000),
    ])];
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let err = deserialize_message_parameters(&mut bytes, 1, ControlMessageType::Fetch).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn test_roundtrip_switch_from() {
    for publish_done in [false, true] {
      for mode in [SwitchMode::Hard, SwitchMode::Soft] {
        let params = vec![MessageParameter::new_switch_from(9, mode, publish_done)];
        let mut bytes = serialize_message_parameters(&params).unwrap();
        let decoded =
          deserialize_message_parameters(&mut bytes, 1, ControlMessageType::Subscribe).unwrap();
        assert_eq!(decoded, params);
      }
    }
  }

  #[test]
  fn test_switch_from_wire_format() {
    let wire = Bytes::from_static(&[0x24, 0x03, 0x07, 0x00, 0x80]);
    let params = vec![MessageParameter::new_switch_from(7, SwitchMode::Hard, true)];

    assert_eq!(serialize_message_parameters(&params).unwrap(), wire);

    let mut buf = wire.clone();
    let decoded =
      deserialize_message_parameters(&mut buf, 1, ControlMessageType::Subscribe).unwrap();
    assert_eq!(decoded, params);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_switch_from_round_trips_in_a_parameter_list() {
    let params = vec![
      MessageParameter::new_subscriber_priority(128),
      MessageParameter::new_switch_from(7, SwitchMode::Hard, true),
      MessageParameter::new_group_request(3),
    ];
    let count = params.len() as u64;
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let decoded =
      deserialize_message_parameters(&mut bytes, count, ControlMessageType::Subscribe).unwrap();

    assert!(!bytes.has_remaining(), "the list left trailing bytes");
    assert_eq!(decoded, params);
  }

  #[test]
  fn test_switch_from_reserved_bits_must_be_zero() {
    let mut value = BytesMut::new();
    value.put_vi(1u64).unwrap();
    value.put_vi(SwitchMode::Hard as u64).unwrap();
    value.put_u8(0x01);
    let kvp =
      KeyValuePair::new_bytes(MessageParameterType::SwitchFrom as u64, value.freeze()).unwrap();
    assert!(matches!(
      MessageParameter::deserialize(&kvp),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn test_switch_from_unknown_mode_is_rejected() {
    let mut value = BytesMut::new();
    value.put_vi(1u64).unwrap();
    value.put_vi(9u64).unwrap();
    value.put_u8(0);
    let kvp =
      KeyValuePair::new_bytes(MessageParameterType::SwitchFrom as u64, value.freeze()).unwrap();
    assert!(MessageParameter::deserialize(&kvp).is_err());
  }

  #[test]
  fn test_switch_from_rejected_outside_subscribe_and_request_update() {
    let params = vec![MessageParameter::new_switch_from(
      1,
      SwitchMode::Hard,
      false,
    )];
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let err = deserialize_message_parameters(&mut bytes, 1, ControlMessageType::Fetch).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));

    for msg_type in [
      ControlMessageType::Subscribe,
      ControlMessageType::RequestUpdate,
    ] {
      let mut bytes = serialize_message_parameters(&params).unwrap();
      assert!(deserialize_message_parameters(&mut bytes, 1, msg_type).is_ok());
    }
  }

  #[test]
  fn test_unknown_type_is_protocol_violation() {
    let kvp = KeyValuePair::try_new_varint(998, 1).unwrap();
    let err = MessageParameter::deserialize(&kvp).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn test_bulk_deserialize_rejects_wrong_message_params() {
    // A parameter in a message type it is not defined for MUST close the
    // session with PROTOCOL_VIOLATION. ObjectDeliveryTimeout is not valid in FETCH.
    let params = vec![
      MessageParameter::new_object_delivery_timeout(100),
      MessageParameter::new_subscriber_priority(50),
    ];
    let param_count = params.len() as u64;
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let err = deserialize_message_parameters(&mut bytes, param_count, ControlMessageType::Fetch)
      .unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn test_fill_timeout_rejected_outside_fetch() {
    // FILL_TIMEOUT is FETCH-only; in a SUBSCRIBE it must be rejected.
    let params = vec![MessageParameter::new_fill_timeout(3000)];
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let err =
      deserialize_message_parameters(&mut bytes, 1, ControlMessageType::Subscribe).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));

    // And accepted in a FETCH.
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let ok = deserialize_message_parameters(&mut bytes, 1, ControlMessageType::Fetch).unwrap();
    assert_eq!(ok, vec![MessageParameter::new_fill_timeout(3000)]);
  }

  #[test]
  fn test_delivery_timeout_zero_means_no_timeout() {
    // A value of 0 is valid and means no timeout.
    let params = vec![MessageParameter::new_object_delivery_timeout(0)];
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let ok = deserialize_message_parameters(&mut bytes, 1, ControlMessageType::Subscribe).unwrap();
    assert_eq!(ok, vec![MessageParameter::new_object_delivery_timeout(0)]);
  }

  #[test]
  fn test_track_namespace_prefix_round_trips_in_a_parameter_list() {
    // Alongside a neighbour on each side, so a desync in the length-prefixed value
    // shows up as a corrupted list rather than passing unnoticed.
    let prefix = Tuple::from_utf8_path("meet/room1/sub");
    let params = vec![
      MessageParameter::new_subscriber_priority(200),
      MessageParameter::new_track_namespace_prefix(prefix.clone()),
    ];
    let count = params.len() as u64;
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let decoded =
      deserialize_message_parameters(&mut bytes, count, ControlMessageType::RequestUpdate).unwrap();

    assert!(!bytes.has_remaining(), "the list left trailing bytes");
    assert_eq!(
      decoded.get_param(MessageParameterType::TrackNamespacePrefix),
      Some(&MessageParameter::new_track_namespace_prefix(prefix))
    );
    assert_eq!(
      decoded.get_param(MessageParameterType::SubscriberPriority),
      Some(&MessageParameter::new_subscriber_priority(200))
    );
  }

  #[test]
  fn test_track_namespace_prefix_rejected_outside_request_update() {
    let params = vec![MessageParameter::new_track_namespace_prefix(
      Tuple::from_utf8_path("meet"),
    )];
    let mut bytes = serialize_message_parameters(&params).unwrap();
    let err = deserialize_message_parameters(&mut bytes, 1, ControlMessageType::SubscribeNamespace)
      .unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn test_track_namespace_prefix_rejects_a_short_field_count() {
    // A count that leaves fields unread must not silently truncate the prefix.
    let mut value = BytesMut::new();
    value.put_vi(1u64).unwrap(); // claims one field
    value.extend_from_slice(&TupleField::from_utf8("meet").serialize().unwrap());
    value.extend_from_slice(&TupleField::from_utf8("room1").serialize().unwrap());

    let kvp = KeyValuePair::Bytes {
      type_value: MessageParameterType::TrackNamespacePrefix as u64,
      value: value.freeze(),
    };
    let err = MessageParameter::deserialize(&kvp).unwrap_err();
    assert!(matches!(err, ParseError::KeyValueFormattingError { .. }));
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
      MessageParameter::new_object_delivery_timeout(500),
    ];
    apply_message_parameter_update(&mut current, updates);
    assert_eq!(current.len(), 3);
    assert!(current.contains(&MessageParameter::new_subscriber_priority(50)));
    assert!(current.contains(&MessageParameter::new_forward(true)));
    assert!(current.contains(&MessageParameter::new_object_delivery_timeout(500)));
  }

  #[test]
  fn test_is_valid_for() {
    let timeout = MessageParameter::new_object_delivery_timeout(100);
    assert!(timeout.is_valid_for(ControlMessageType::Subscribe));
    assert!(timeout.is_valid_for(ControlMessageType::PublishOk));
    assert!(!timeout.is_valid_for(ControlMessageType::Fetch));
  }

  #[test]
  fn test_type_value() {
    assert_eq!(
      MessageParameter::new_object_delivery_timeout(0).type_value(),
      MessageParameterType::ObjectDeliveryTimeout as u64
    );
    assert_eq!(
      MessageParameter::new_forward(true).type_value(),
      MessageParameterType::Forward as u64
    );
  }

  #[test]
  fn test_bug_report_wire_format_is_delta_encoded() {
    // Regression for the reported interop bug: SUBSCRIBER_PRIORITY (0x20),
    // FORWARD (0x10) and SUBSCRIPTION_FILTER (0x21), built in non-ascending
    // insertion order. A spec-compliant v16 peer decodes Type as a delta from
    // the previous Type in the list; encoding them "as-is" (absolute) made a
    // correct delta-decoder compute types 48 and 81 instead.
    let params = vec![
      MessageParameter::new_subscriber_priority(0),
      MessageParameter::new_forward(true),
      MessageParameter::new_subscription_filter(FilterType::LatestObject, None, None),
    ];
    let mut bytes = serialize_message_parameters(&params).unwrap();

    // Decode independently of deserialize_message_parameters, using raw delta
    // semantics, to prove the wire bytes are genuinely delta-encoded and not
    // just self-consistent with our own (potentially still-buggy) decoder.
    let mut prev_type = 0u64;
    let mut types = Vec::new();
    while bytes.has_remaining() {
      let kvp = KeyValuePair::deserialize_delta(&mut bytes, prev_type).unwrap();
      prev_type = kvp.get_type();
      types.push(prev_type);
    }
    assert_eq!(
      types,
      vec![
        MessageParameterType::Forward as u64,
        MessageParameterType::SubscriberPriority as u64,
        MessageParameterType::SubscriptionFilter as u64,
      ]
    );
  }

  #[test]
  fn test_decode_delta_encoded_peer_stream() {
    // Simulates a spec-compliant peer sending FORWARD then SUBSCRIBER_PRIORITY,
    // correctly delta-encoded. This is the direction the reporter's own
    // workaround (disabling delta decoding) broke.
    let mut buf = BytesMut::new();
    buf.put_vi(MessageParameterType::Forward as u64).unwrap(); // delta from 0 -> 0x10
    buf.put_vi(1u64).unwrap(); // true
    buf
      .put_vi(
        MessageParameterType::SubscriberPriority as u64 - MessageParameterType::Forward as u64,
      )
      .unwrap(); // delta from 0x10 -> 0x20
    buf.put_vi(5u64).unwrap();
    let mut bytes = buf.freeze();

    let params =
      deserialize_message_parameters(&mut bytes, 2, ControlMessageType::Subscribe).unwrap();
    assert_eq!(
      params,
      vec![
        MessageParameter::new_forward(true),
        MessageParameter::new_subscriber_priority(5),
      ]
    );
  }
}
