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
use crate::model::control::constant::{ControlMessageType, GroupOrder, SwitchMode};
use crate::model::error::ParseError;
use crate::model::parameter::authorization_token::AuthorizationToken;
use crate::model::parameter::constant::MessageParameterType;
use crate::model::parameter::location_filter::LocationFilter;
use crate::model::parameter::range_filter::{RangeFilter, RangeFilterKind};
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
  LocationFilter {
    filter: LocationFilter,
  },
  SwitchFrom {
    /// The subscription to suspend.
    request_id: u64,
    mode: SwitchMode,
    /// Whether the publisher sends PUBLISH_DONE on the suspended subscription.
    publish_done: bool,
  },
  /// Requests a fill fetch stream, and overrides for it. An omitted parameter
  /// keeps the value it has for the live subscription; an omitted LOCATION_FILTER
  /// fills over the subscription's own.
  FillParameters {
    parameters: Vec<MessageParameter>,
  },
  /// One Range Filter. Unlike other parameters it may repeat in a message, once
  /// per SetID (and Property Type). `None` is the zero-length form, which in
  /// REQUEST_UPDATE removes the filters of that kind.
  RangeFilter {
    kind: RangeFilterKind,
    filter: Option<RangeFilter>,
  },
  NewGroupRequest {
    group: u64,
  },
  TrackNamespacePrefix {
    prefix: Tuple,
  },
  /// Whether the response carries Track Properties: the OK answering a SUBSCRIBE,
  /// TRACK_STATUS or FETCH, or each PUBLISH a SUBSCRIBE_TRACKS brings. Omitted means
  /// they are included.
  IncludeProperties {
    include: bool,
  },
  /// Assigns a subscription to an SSTS switching set. Id 0 is the default
  /// allocation every implementation runs; other ids select an algorithm
  /// specific to one implementation.
  SwitchingSetAssignment {
    switching_set_id: u64,
    algorithm_id: u64,
    throughput_threshold_kbps: u64,
    /// Relative bandwidth weight among same-rank sets; 1 <= N <= 10.
    set_throughput_weight: u64,
    /// 0 pauses SSTS for the set; switching activates once the number of
    /// assigned tracks is >= this value.
    activate_switching: u64,
    /// Degradation priority; lower values are protected first. Default 0.
    set_rank: u8,
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

  pub fn new_include_properties(include: bool) -> Self {
    Self::IncludeProperties { include }
  }

  pub fn new_subscriber_priority(priority: u8) -> Self {
    Self::SubscriberPriority { priority }
  }

  pub fn new_group_order(order: GroupOrder) -> Self {
    Self::GroupOrder { order }
  }

  pub fn new_location_filter(filter: LocationFilter) -> Self {
    Self::LocationFilter { filter }
  }

  pub fn new_fill_parameters(parameters: Vec<MessageParameter>) -> Self {
    Self::FillParameters { parameters }
  }

  pub fn new_range_filter(kind: RangeFilterKind, filter: RangeFilter) -> Self {
    Self::RangeFilter {
      kind,
      filter: Some(filter),
    }
  }

  /// The zero-length form, which removes the filters of `kind`.
  pub fn new_range_filter_removal(kind: RangeFilterKind) -> Self {
    Self::RangeFilter { kind, filter: None }
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

  pub fn new_switching_set_assignment(
    switching_set_id: u64,
    algorithm_id: u64,
    throughput_threshold_kbps: u64,
    set_throughput_weight: u64,
    activate_switching: u64,
    set_rank: u8,
  ) -> Self {
    Self::SwitchingSetAssignment {
      switching_set_id,
      algorithm_id,
      throughput_threshold_kbps,
      set_throughput_weight,
      activate_switching,
      set_rank,
    }
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
      Self::LocationFilter { .. } => MessageParameterType::LocationFilter as u64,
      Self::FillParameters { .. } => MessageParameterType::FillParameters as u64,
      Self::SwitchFrom { .. } => MessageParameterType::SwitchFrom as u64,
      Self::RangeFilter { kind, .. } => kind.parameter_type() as u64,
      Self::NewGroupRequest { .. } => MessageParameterType::NewGroupRequest as u64,
      Self::TrackNamespacePrefix { .. } => MessageParameterType::TrackNamespacePrefix as u64,
      Self::IncludeProperties { .. } => MessageParameterType::IncludeProperties as u64,
      Self::SwitchingSetAssignment { .. } => MessageParameterType::SwitchingSetAssignment as u64,
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
      // Subscription parameters ride the request that sets up or changes a
      // subscription, never the PUBLISH_OK (REQUEST_OK) that accepts one.
      Self::ObjectDeliveryTimeout { .. } | Self::SubgroupDeliveryTimeout { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe | ControlMessageType::RequestUpdate
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
      ),
      Self::GroupOrder { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::SubscribeTracks
          | ControlMessageType::Fetch
          | ControlMessageType::Publish
      ),
      Self::LocationFilter { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::Fetch
          | ControlMessageType::Publish
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
          | ControlMessageType::SubscribeNamespace
          | ControlMessageType::SubscribeTracks
      ),
      Self::NewGroupRequest { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe | ControlMessageType::RequestUpdate
      ),
      Self::IncludeProperties { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::TrackStatus
          | ControlMessageType::Fetch
          | ControlMessageType::SubscribeTracks
      ),
      // Only a subscriber sends filters, and a track filter only means something
      // where tracks are being chosen.
      Self::RangeFilter {
        kind: RangeFilterKind::TrackProperty,
        ..
      } => matches!(
        msg_type,
        ControlMessageType::SubscribeTracks | ControlMessageType::RequestUpdate
      ),
      Self::RangeFilter { .. } => matches!(
        msg_type,
        ControlMessageType::Subscribe
          | ControlMessageType::Fetch
          | ControlMessageType::SubscribeTracks
          | ControlMessageType::RequestUpdate
      ),
      Self::TrackNamespacePrefix { .. } => matches!(msg_type, ControlMessageType::RequestUpdate),
      // A switching set is joined when the subscription is created and
      // re-tuned afterwards, so the parameter rides SUBSCRIBE, REQUEST_UPDATE
      // and the PUBLISH_OK that accepts a pushed track — which arrives here as
      // REQUEST_OK.
      Self::SwitchingSetAssignment { .. } => matches!(
        msg_type,
        ControlMessageType::PublishOk
          | ControlMessageType::RequestOk
          | ControlMessageType::Subscribe
          | ControlMessageType::RequestUpdate
      ),
    }
  }

  /// Encodes this parameter as it appears on the wire on its own: its Type, then
  /// its Value in the encoding the Type defines.
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    serialize_message_parameters(std::slice::from_ref(self))
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
          MessageParameterType::IncludeProperties => match *value {
            0 => Ok(Self::IncludeProperties { include: false }),
            1 => Ok(Self::IncludeProperties { include: true }),
            _ => Err(ParseError::ProtocolViolation {
              context: "MessageParameter::deserialize",
              details: format!("INCLUDE_PROPERTIES must be 0 or 1, got {value}"),
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
          MessageParameterType::SwitchingSetAssignment => {
            let mut payload = value.clone();
            let switching_set_id = payload.get_vi()?;
            let algorithm_id = payload.get_vi()?;
            let throughput_threshold_kbps = payload.get_vi()?;
            let set_throughput_weight = payload.get_vi()?;
            if !(1..=10).contains(&set_throughput_weight) {
              return Err(ParseError::ProtocolViolation {
                context: "MessageParameter::deserialize",
                details: format!(
                  "SET THROUGHPUT WEIGHT must be 1-10, got {}",
                  set_throughput_weight
                ),
              });
            }
            let activate_switching = payload.get_vi()?;
            let set_rank = if payload.has_remaining() {
              payload.get_u8()
            } else {
              0 // Default
            };
            if payload.has_remaining() {
              return Err(ParseError::ProtocolViolation {
                context: "MessageParameter::deserialize",
                details: "Excess bytes in SWITCHING_SET_ASSIGNMENT parameter".to_string(),
              });
            }

            Ok(Self::SwitchingSetAssignment {
              switching_set_id,
              algorithm_id,
              throughput_threshold_kbps,
              set_throughput_weight,
              activate_switching,
              set_rank,
            })
          }
          MessageParameterType::SubgroupFilter
          | MessageParameterType::ObjectIdFilter
          | MessageParameterType::PriorityFilter
          | MessageParameterType::ObjectPropertyFilter
          | MessageParameterType::TrackPropertyFilter => {
            let kind = RangeFilterKind::from_parameter_type(param_type)
              .expect("every range filter parameter type has a kind");
            let filter = if value.is_empty() {
              None
            } else {
              Some(RangeFilter::deserialize(kind, &mut value.clone())?)
            };
            Ok(Self::RangeFilter { kind, filter })
          }
          MessageParameterType::LocationFilter => {
            let mut payload = value.clone();
            let filter = LocationFilter::deserialize(&mut payload)?;
            if payload.has_remaining() {
              return Err(ParseError::KeyValueFormattingError {
                context: "MessageParameter::deserialize(LocationFilter)",
              });
            }
            Ok(Self::LocationFilter { filter })
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
      // A uint8 on an odd Type, which the even/odd rule of try_new_varint would refuse;
      // its value shape is what decides the encoding.
      Self::IncludeProperties { include } => Ok(KeyValuePair::VarInt {
        type_value: MessageParameterType::IncludeProperties as u64,
        value: if include { 1 } else { 0 },
      }),
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
      Self::SwitchingSetAssignment {
        switching_set_id,
        algorithm_id,
        throughput_threshold_kbps,
        set_throughput_weight,
        activate_switching,
        set_rank,
      } => {
        let mut buf = BytesMut::new();
        buf.put_vi(switching_set_id)?;
        buf.put_vi(algorithm_id)?;
        buf.put_vi(throughput_threshold_kbps)?;
        buf.put_vi(set_throughput_weight)?;
        buf.put_vi(activate_switching)?;
        buf.put_u8(set_rank);

        KeyValuePair::try_new_bytes(
          MessageParameterType::SwitchingSetAssignment as u64,
          buf.freeze(),
        )
      }
      // Not length-prefixed: the Location Filter Type says how many fields follow.
      Self::LocationFilter { filter } => KeyValuePair::new_bytes(
        MessageParameterType::LocationFilter as u64,
        filter.serialize()?,
      ),
      // Length-prefixed whatever the Type's parity; an empty Value removes.
      Self::RangeFilter { kind, filter } => {
        let value = match filter {
          Some(filter) => {
            // A Property Type where the kind has none, or none where it has one,
            // would be read back as a Range.
            if kind.has_property_type() != filter.property_type.is_some() {
              return Err(ParseError::ProtocolViolation {
                context: "MessageParameter::try_into(RangeFilter)",
                details: format!("{kind:?} filter has the wrong Property Type presence"),
              });
            }
            filter.serialize()?
          }
          None => Bytes::new(),
        };
        KeyValuePair::new_bytes(kind.parameter_type() as u64, value)
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
/// Whether a request's response should carry Track Properties: false only when it
/// has INCLUDE_PROPERTIES set to 0.
pub fn include_properties(params: &[MessageParameter]) -> bool {
  !params
    .iter()
    .any(|p| matches!(p, MessageParameter::IncludeProperties { include: false }))
}

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
  /// A Location Filter Type and the varints it calls for, no length prefix.
  LocationFilter,
  LengthPrefixedBytes,
}

/// The only parameters that may override anything for the fill fetch stream.
/// A track filter is not among them: the fill is for a track already chosen.
const fn is_valid_fill_parameter(type_value: u64) -> bool {
  type_value == MessageParameterType::FillTimeout as u64
    || type_value == MessageParameterType::SubscriberPriority as u64
    || type_value == MessageParameterType::LocationFilter as u64
    || type_value == MessageParameterType::GroupOrder as u64
    || is_object_range_filter(type_value)
}

const fn is_object_range_filter(type_value: u64) -> bool {
  type_value == MessageParameterType::SubgroupFilter as u64
    || type_value == MessageParameterType::ObjectIdFilter as u64
    || type_value == MessageParameterType::PriorityFilter as u64
    || type_value == MessageParameterType::ObjectPropertyFilter as u64
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
    // Decoded here because nothing else knows where it ends, and re-encoded so the
    // parameter is decoded the same way as one built locally.
    ValueShape::LocationFilter => {
      KeyValuePair::new_bytes(type_value, LocationFilter::deserialize(bytes)?.serialize()?)
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
    || type_value == MessageParameterType::IncludeProperties as u64
  {
    ValueShape::Uint8
  } else if type_value == MessageParameterType::LargestObject as u64 {
    ValueShape::BareLocation
  } else if type_value == MessageParameterType::LocationFilter as u64 {
    ValueShape::LocationFilter
  } else if type_value == MessageParameterType::TrackNamespacePrefix as u64
    || type_value == MessageParameterType::SwitchFrom as u64
    || is_object_range_filter(type_value)
    || type_value == MessageParameterType::TrackPropertyFilter as u64
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
      (
        ValueShape::BareLocation | ValueShape::LocationFilter,
        KeyValuePair::Bytes { value, .. },
      ) => {
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
///
/// A Type the update leaves out keeps its value. A Type it carries is replaced
/// outright, every instance of it: a Range Filter can repeat, and an update that
/// names a filter kind states the whole of that kind, so one left over from
/// before would quietly widen or narrow what the subscriber asked for. A
/// zero-length Range Filter replaces its kind with nothing.
pub fn apply_message_parameter_update(
  current: &mut Vec<MessageParameter>,
  updates: Vec<MessageParameter>,
) {
  current.retain(|p| !updates.iter().any(|u| u.type_value() == p.type_value()));
  current.extend(
    updates
      .into_iter()
      .filter(|u| !matches!(u, MessageParameter::RangeFilter { filter: None, .. })),
  );
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::tuple::TupleField;
  use crate::model::control::constant::LocationFilterType;
  use crate::model::parameter::range_filter::Range;

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
    let mut bytes = param.serialize().unwrap();
    let type_value = bytes.get_vi().unwrap();
    let kvp = read_message_parameter_value(&mut bytes, type_value).unwrap();
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
  fn test_roundtrip_switching_set_assignment() {
    let orig = MessageParameter::new_switching_set_assignment(7, 0, 2000, 5, 2, 2);
    assert_eq!(roundtrip(orig.clone()), orig);

    let orig = MessageParameter::new_switching_set_assignment(0, 0, 0, 1, 0, 0);
    assert_eq!(roundtrip(orig.clone()), orig);
  }

  /// SWITCHING_SET_ASSIGNMENT comes from an unadopted draft, so nothing else in
  /// the stack constrains its fields: a relay that accepted a weight outside
  /// 1-10 would carry a set property that no allocation can honour.
  #[test]
  fn switching_set_assignment_weight_must_be_one_to_ten() {
    for weight in [0u64, 11, 1000] {
      let payload = assignment_payload(&[7, 0, 100, weight, 2, 0]);
      let error = MessageParameter::deserialize(&payload)
        .err()
        .unwrap_or_else(|| panic!("weight {weight} must be rejected"));
      assert!(
        error.to_string().contains("SET THROUGHPUT WEIGHT"),
        "the rejection should say which field was wrong, got {error}"
      );
    }
    // The ends of the range are inclusive.
    for weight in [1u64, 10] {
      let payload = assignment_payload(&[7, 0, 100, weight, 2, 0]);
      MessageParameter::deserialize(&payload).unwrap_or_else(|e| panic!("weight {weight}: {e}"));
    }
  }

  /// SET RANK is optional on the wire; a peer that leaves it out means the
  /// highest priority tier, and must not be treated as a protocol error.
  #[test]
  fn switching_set_assignment_rank_defaults_when_omitted() {
    let payload = assignment_payload(&[7, 0, 100, 5, 2]);
    assert_eq!(
      MessageParameter::deserialize(&payload).unwrap(),
      MessageParameter::new_switching_set_assignment(7, 0, 100, 5, 2, 0)
    );
  }

  #[test]
  fn switching_set_assignment_rejects_trailing_bytes() {
    // A parameter longer than its fields means the peer and the relay
    // disagree about the format; guessing would silently mis-assign a set.
    let payload = assignment_payload(&[7, 0, 100, 5, 2, 1, 0xAA]);
    let error = MessageParameter::deserialize(&payload).unwrap_err();
    assert!(error.to_string().contains("Excess bytes"), "got {error}");
  }

  #[test]
  fn switching_set_assignment_only_appears_where_the_draft_allows_it() {
    let param = MessageParameter::new_switching_set_assignment(7, 0, 100, 5, 2, 0);
    for msg_type in [
      ControlMessageType::Subscribe,
      ControlMessageType::RequestUpdate,
      ControlMessageType::PublishOk,
      ControlMessageType::RequestOk,
    ] {
      assert!(param.is_valid_for(msg_type), "{msg_type:?} allows it");
    }
    for msg_type in [
      ControlMessageType::Publish,
      ControlMessageType::PublishDone,
      ControlMessageType::SubscribeOk,
      ControlMessageType::GoAway,
    ] {
      assert!(!param.is_valid_for(msg_type), "{msg_type:?} must not");
    }
  }

  /// Build a parameter's KeyValuePair straight from bytes, so a test can put a
  /// field sequence on the wire that this crate's own serializer would never
  /// produce.
  fn assignment_payload(fields: &[u64]) -> KeyValuePair {
    let mut payload = BytesMut::new();
    for field in fields {
      payload.put_vi(*field).unwrap();
    }
    KeyValuePair::try_new_bytes(
      MessageParameterType::SwitchingSetAssignment as u64,
      payload.freeze(),
    )
    .unwrap()
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

  /// Encodes `params` the way a control message carries them and decodes them
  /// back, checking nothing is left over.
  fn wire_roundtrip(params: &[MessageParameter], msg_type: ControlMessageType) {
    let mut bytes = serialize_message_parameters(params).unwrap();
    let decoded =
      deserialize_message_parameters(&mut bytes, params.len() as u64, msg_type).unwrap();
    assert_eq!(decoded, params);
    assert!(!bytes.has_remaining(), "the list left trailing bytes");
  }

  #[test]
  fn test_roundtrip_location_filter_every_type() {
    for filter in [
      LocationFilter::None,
      LocationFilter::RelativeStart { start_group: 2 },
      LocationFilter::AbsoluteStart {
        start: Location::new(3, 1),
      },
      LocationFilter::AbsoluteStartGroupEnd {
        start: Location::new(5, 0),
        end_group: 20,
      },
      LocationFilter::AbsoluteRange {
        start: Location::new(5, 0),
        end: Location::new(20, 9),
      },
      LocationFilter::NextObject,
    ] {
      wire_roundtrip(
        &[MessageParameter::new_location_filter(filter)],
        ControlMessageType::Subscribe,
      );
    }
  }

  #[test]
  fn test_location_filter_wire_format_has_no_length() {
    // Parameter Type 0x21, then the Location Filter Type and its fields only.
    let cases: [(LocationFilter, &[u8]); 3] = [
      (LocationFilter::NextObject, &[0x21, 0x05]),
      (
        LocationFilter::RelativeStart { start_group: 1 },
        &[0x21, 0x01, 0x01],
      ),
      (
        LocationFilter::AbsoluteRange {
          start: Location::new(5, 0),
          end: Location::new(20, 3),
        },
        // End Group 20 travels as the delta 15 from Start Group 5.
        &[0x21, 0x04, 0x05, 0x00, 0x0F, 0x03],
      ),
    ];
    for (filter, wire) in cases {
      let params = vec![MessageParameter::new_location_filter(filter)];
      assert_eq!(
        serialize_message_parameters(&params).unwrap().as_ref(),
        wire
      );

      let mut buf = Bytes::copy_from_slice(wire);
      let decoded =
        deserialize_message_parameters(&mut buf, 1, ControlMessageType::Subscribe).unwrap();
      assert_eq!(decoded, params);
      assert!(!buf.has_remaining());
    }
  }

  #[test]
  fn test_location_filter_ends_where_its_type_says_in_a_list() {
    // A field-less filter type followed by another parameter: the decoder must
    // not mistake the next parameter's Type Delta for a length or a field.
    let params = vec![
      MessageParameter::new_subscriber_priority(7),
      MessageParameter::new_location_filter(LocationFilter::NextObject),
      MessageParameter::new_group_order(GroupOrder::Descending),
    ];
    let wire: &[u8] = &[0x20, 0x07, 0x01, 0x05, 0x01, 0x02];
    assert_eq!(
      serialize_message_parameters(&params).unwrap().as_ref(),
      wire
    );
    wire_roundtrip(&params, ControlMessageType::Subscribe);
  }

  #[test]
  fn test_location_filter_unknown_type_is_protocol_violation() {
    let mut buf = Bytes::from_static(&[0x21, 0x06]);
    assert!(matches!(
      deserialize_message_parameters(&mut buf, 1, ControlMessageType::Subscribe),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn test_location_filter_end_group_overflow_is_protocol_violation() {
    let mut value = BytesMut::new();
    value
      .put_vi(LocationFilterType::AbsoluteStartGroupEnd as u64)
      .unwrap();
    value.put_vi(u64::MAX).unwrap(); // start group
    value.put_vi(0u64).unwrap(); // start object
    value.put_vi(1u64).unwrap(); // EndGroupDelta -> overflow
    let kvp =
      KeyValuePair::new_bytes(MessageParameterType::LocationFilter as u64, value.freeze()).unwrap();
    assert!(matches!(
      MessageParameter::deserialize(&kvp),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn test_location_filter_truncated_is_an_error() {
    // Absolute Start needs StartGroup and StartObject; only one follows.
    let mut buf = Bytes::from_static(&[0x21, 0x02, 0x05]);
    assert!(deserialize_message_parameters(&mut buf, 1, ControlMessageType::Subscribe).is_err());
  }

  #[test]
  fn test_a_single_parameter_serializes_in_its_wire_encoding() {
    // Not the generic key-value encoding, which would length-prefix the filter
    // and write the priority as a two-byte varint.
    assert_eq!(
      MessageParameter::new_location_filter(LocationFilter::NextObject)
        .serialize()
        .unwrap()
        .as_ref(),
      &[0x21, 0x05]
    );
    assert_eq!(
      MessageParameter::new_subscriber_priority(128)
        .serialize()
        .unwrap()
        .as_ref(),
      &[0x20, 0x80]
    );
  }

  #[test]
  fn test_location_filter_may_ride_a_publish() {
    wire_roundtrip(
      &[MessageParameter::new_location_filter(
        LocationFilter::NextObject,
      )],
      ControlMessageType::Publish,
    );
  }

  #[test]
  fn test_location_filter_is_allowed_inside_fill_parameters() {
    let params = vec![MessageParameter::new_fill_parameters(vec![
      MessageParameter::new_location_filter(LocationFilter::RelativeStart { start_group: 1 }),
    ])];
    // FILL_PARAMETERS 0x23, length 3, then LOCATION_FILTER 0x21 type 1 StartGroup 1.
    let wire: &[u8] = &[0x23, 0x03, 0x21, 0x01, 0x01];
    assert_eq!(
      serialize_message_parameters(&params).unwrap().as_ref(),
      wire
    );
    wire_roundtrip(&params, ControlMessageType::Subscribe);
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
    assert!(timeout.is_valid_for(ControlMessageType::RequestUpdate));
    assert!(!timeout.is_valid_for(ControlMessageType::Fetch));
  }

  #[test]
  fn subscription_parameters_ride_request_update_not_publish_ok() {
    for param in [
      MessageParameter::new_object_delivery_timeout(100),
      MessageParameter::new_subgroup_delivery_timeout(100),
      MessageParameter::new_subscriber_priority(7),
      MessageParameter::new_location_filter(LocationFilter::NextObject),
      MessageParameter::new_forward(false),
      MessageParameter::new_group_request(3),
    ] {
      assert!(
        param.is_valid_for(ControlMessageType::RequestUpdate),
        "{param:?}"
      );
      for msg_type in [ControlMessageType::PublishOk, ControlMessageType::RequestOk] {
        assert!(!param.is_valid_for(msg_type), "{param:?} in {msg_type:?}");
      }
    }
    // What a PUBLISH_OK or REQUEST_UPDATE_OK may still carry.
    assert!(MessageParameter::new_expires(10).is_valid_for(ControlMessageType::RequestOk));
    assert!(
      MessageParameter::new_largest_object(Location::new(1, 2))
        .is_valid_for(ControlMessageType::RequestOk)
    );
  }

  #[test]
  fn a_publish_ok_carrying_forward_is_a_protocol_violation() {
    let mut bytes = serialize_message_parameters(&[MessageParameter::new_forward(true)]).unwrap();
    assert!(matches!(
      deserialize_message_parameters(&mut bytes, 1, ControlMessageType::RequestOk),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn test_roundtrip_include_properties() {
    for include in [false, true] {
      let orig = MessageParameter::new_include_properties(include);
      assert_eq!(roundtrip(orig.clone()), orig);
    }
  }

  #[test]
  fn include_properties_is_a_single_byte() {
    let bytes =
      serialize_message_parameters(&[MessageParameter::new_include_properties(false)]).unwrap();
    assert_eq!(bytes.as_ref(), &[0x35, 0x00]);
  }

  #[test]
  fn include_properties_other_than_0_or_1_is_a_protocol_violation() {
    let mut bytes = Bytes::from_static(&[0x35, 0x02]);
    assert!(matches!(
      deserialize_message_parameters(&mut bytes, 1, ControlMessageType::Subscribe),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn include_properties_rides_the_requests_whose_response_carries_properties() {
    let param = MessageParameter::new_include_properties(false);
    for msg_type in [
      ControlMessageType::Subscribe,
      ControlMessageType::TrackStatus,
      ControlMessageType::Fetch,
      ControlMessageType::SubscribeTracks,
    ] {
      assert!(param.is_valid_for(msg_type), "{msg_type:?} allows it");
    }
    for msg_type in [
      ControlMessageType::RequestUpdate,
      ControlMessageType::Publish,
      ControlMessageType::SubscribeOk,
      ControlMessageType::RequestOk,
    ] {
      assert!(!param.is_valid_for(msg_type), "{msg_type:?} must not");
    }
  }

  #[test]
  fn properties_are_included_unless_asked_not_to_be() {
    assert!(include_properties(&[]));
    assert!(include_properties(&[
      MessageParameter::new_include_properties(true)
    ]));
    assert!(!include_properties(&[
      MessageParameter::new_include_properties(false)
    ]));
  }

  #[test]
  fn group_order_is_only_asked_for_never_answered() {
    // The order is the requester's to state, in the message that asks for the
    // Objects. A response has nothing to add: the subscriber already chose, and
    // an omitted order already means the publisher's own.
    let order = MessageParameter::new_group_order(GroupOrder::Descending);
    for msg_type in [
      ControlMessageType::Subscribe,
      ControlMessageType::SubscribeTracks,
      ControlMessageType::Publish,
      ControlMessageType::Fetch,
    ] {
      assert!(order.is_valid_for(msg_type), "{msg_type:?} allows it");
    }
    for msg_type in [
      ControlMessageType::PublishOk,
      ControlMessageType::RequestOk,
      ControlMessageType::SubscribeOk,
      ControlMessageType::FetchOk,
    ] {
      assert!(!order.is_valid_for(msg_type), "{msg_type:?} must not");
    }
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
    // FORWARD (0x10) and LOCATION_FILTER (0x21), built in non-ascending
    // insertion order. A spec-compliant v16 peer decodes Type as a delta from
    // the previous Type in the list; encoding them "as-is" (absolute) made a
    // correct delta-decoder compute types 48 and 81 instead.
    let params = vec![
      MessageParameter::new_subscriber_priority(0),
      MessageParameter::new_forward(true),
      MessageParameter::new_location_filter(LocationFilter::NextObject),
    ];
    let bytes = serialize_message_parameters(&params).unwrap();

    // Checked against the bytes themselves rather than our own decoder, to prove
    // the Types are genuinely delta-encoded and not just self-consistent with a
    // (potentially still-buggy) decoder.
    assert_eq!(
      bytes.as_ref(),
      &[
        0x10, 0x01, // FORWARD (delta 0x10) = 1
        0x10, 0x00, // SUBSCRIBER_PRIORITY (delta 0x10 -> 0x20) = 0
        0x01, 0x05, // LOCATION_FILTER (delta 0x01 -> 0x21), Next Object
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

  fn range_filter(kind: RangeFilterKind, set_id: u8, ranges: &[Range]) -> MessageParameter {
    let property_type = kind.has_property_type().then_some(0x3C);
    MessageParameter::new_range_filter(
      kind,
      RangeFilter::new(set_id, property_type, ranges).unwrap(),
    )
  }

  #[test]
  fn an_even_typed_range_filter_is_length_prefixed() {
    // OBJECTID_FILTER (0x26) is even, yet length-prefixed. Read as a varint, its
    // Length would be taken for the Value and the rest of the list would desync.
    let params = vec![
      range_filter(RangeFilterKind::ObjectId, 0, &[Range::new(3, 5)]),
      MessageParameter::new_group_request(9),
    ];
    let wire = serialize_message_parameters(&params).unwrap();
    assert_eq!(
      wire.as_ref(),
      &[
        0x26, 0x03, 0x00, 0x03, 0x02, // OBJECTID_FILTER, Length 3, SetID 0, 3-5
        0x0C, 0x09, // NEW_GROUP_REQUEST (delta 0x0C -> 0x32) = 9
      ]
    );
    let mut buf = wire.clone();
    let decoded =
      deserialize_message_parameters(&mut buf, 2, ControlMessageType::Subscribe).unwrap();
    assert_eq!(decoded, params);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn range_filters_repeat_in_one_message() {
    let params = vec![
      range_filter(RangeFilterKind::Subgroup, 0, &[Range::new(0, 1)]),
      range_filter(RangeFilterKind::Subgroup, 1, &[Range::open(4)]),
      range_filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 0)]),
      range_filter(RangeFilterKind::Priority, 1, &[Range::new(0, 127)]),
      range_filter(RangeFilterKind::ObjectProperty, 0, &[Range::new(10, 20)]),
      range_filter(RangeFilterKind::ObjectProperty, 1, &[Range::new(30, 40)]),
    ];
    let mut wire = serialize_message_parameters(&params).unwrap();
    let decoded = deserialize_message_parameters(
      &mut wire,
      params.len() as u64,
      ControlMessageType::Subscribe,
    )
    .unwrap();
    assert_eq!(decoded, params);
  }

  #[test]
  fn a_zero_length_range_filter_roundtrips_as_a_removal() {
    for kind in RangeFilterKind::ALL {
      let param = MessageParameter::new_range_filter_removal(kind);
      let mut wire = serialize_message_parameters(std::slice::from_ref(&param)).unwrap();
      assert_eq!(wire.as_ref(), &[kind.parameter_type() as u8, 0x00]);
      let decoded =
        deserialize_message_parameters(&mut wire, 1, ControlMessageType::RequestUpdate).unwrap();
      assert_eq!(decoded, vec![param]);
    }
  }

  #[test]
  fn a_property_type_must_be_present_exactly_for_property_filters() {
    let missing = MessageParameter::new_range_filter(
      RangeFilterKind::ObjectProperty,
      RangeFilter::new(0, None, &[Range::new(1, 1)]).unwrap(),
    );
    let stray = MessageParameter::new_range_filter(
      RangeFilterKind::Subgroup,
      RangeFilter::new(0, Some(0x3C), &[Range::new(1, 1)]).unwrap(),
    );
    for param in [missing, stray] {
      assert!(serialize_message_parameters(&[param]).is_err());
    }
  }

  #[test]
  fn object_filters_ride_requests_from_a_subscriber() {
    for kind in [
      RangeFilterKind::Subgroup,
      RangeFilterKind::ObjectId,
      RangeFilterKind::Priority,
      RangeFilterKind::ObjectProperty,
    ] {
      let param = MessageParameter::new_range_filter_removal(kind);
      for msg in [
        ControlMessageType::Subscribe,
        ControlMessageType::Fetch,
        ControlMessageType::SubscribeTracks,
        ControlMessageType::RequestUpdate,
      ] {
        assert!(param.is_valid_for(msg), "{kind:?} in {msg:?}");
      }
      for msg in [
        ControlMessageType::Publish,
        ControlMessageType::SubscribeOk,
        ControlMessageType::RequestOk,
        ControlMessageType::TrackStatus,
      ] {
        assert!(!param.is_valid_for(msg), "{kind:?} in {msg:?}");
      }
    }
  }

  #[test]
  fn a_track_filter_only_rides_subscribe_tracks() {
    let param = MessageParameter::new_range_filter_removal(RangeFilterKind::TrackProperty);
    assert!(param.is_valid_for(ControlMessageType::SubscribeTracks));
    assert!(param.is_valid_for(ControlMessageType::RequestUpdate));
    assert!(!param.is_valid_for(ControlMessageType::Subscribe));
    assert!(!param.is_valid_for(ControlMessageType::Fetch));

    let mut wire = serialize_message_parameters(&[param]).unwrap();
    assert!(matches!(
      deserialize_message_parameters(&mut wire, 1, ControlMessageType::Subscribe),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn fill_parameters_carry_object_filters_but_not_a_track_filter() {
    let fill = MessageParameter::new_fill_parameters(vec![
      range_filter(RangeFilterKind::Subgroup, 0, &[Range::new(0, 0)]),
      range_filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 3)]),
      range_filter(RangeFilterKind::Priority, 0, &[Range::new(0, 64)]),
      range_filter(RangeFilterKind::ObjectProperty, 0, &[Range::new(1, 1)]),
    ]);
    let mut wire = serialize_message_parameters(std::slice::from_ref(&fill)).unwrap();
    let decoded =
      deserialize_message_parameters(&mut wire, 1, ControlMessageType::Subscribe).unwrap();
    assert_eq!(decoded, vec![fill]);

    let track = MessageParameter::new_fill_parameters(vec![range_filter(
      RangeFilterKind::TrackProperty,
      0,
      &[Range::new(1, 1)],
    )]);
    assert!(matches!(
      serialize_message_parameters(&[track]),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }

  #[test]
  fn an_update_replaces_every_filter_of_a_kind_it_names() {
    let mut current = vec![
      range_filter(RangeFilterKind::Subgroup, 0, &[Range::new(0, 0)]),
      range_filter(RangeFilterKind::Subgroup, 1, &[Range::new(5, 5)]),
      range_filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 3)]),
      MessageParameter::new_forward(true),
    ];
    let replacement = range_filter(RangeFilterKind::Subgroup, 2, &[Range::new(9, 9)]);
    apply_message_parameter_update(&mut current, vec![replacement.clone()]);

    let subgroup: Vec<_> = current
      .iter()
      .filter(|p| p.type_value() == MessageParameterType::SubgroupFilter as u64)
      .collect();
    assert_eq!(subgroup, vec![&replacement]);
    // Kinds the update did not name are untouched.
    assert!(
      current
        .iter()
        .any(|p| p.type_value() == MessageParameterType::ObjectIdFilter as u64)
    );
    assert!(current.contains(&MessageParameter::new_forward(true)));
  }

  #[test]
  fn a_zero_length_update_removes_the_kind() {
    let mut current = vec![
      range_filter(RangeFilterKind::Priority, 0, &[Range::new(0, 10)]),
      range_filter(RangeFilterKind::Priority, 1, &[Range::new(20, 30)]),
      range_filter(RangeFilterKind::ObjectId, 0, &[Range::new(0, 3)]),
    ];
    apply_message_parameter_update(
      &mut current,
      vec![MessageParameter::new_range_filter_removal(
        RangeFilterKind::Priority,
      )],
    );
    assert_eq!(
      current,
      vec![range_filter(
        RangeFilterKind::ObjectId,
        0,
        &[Range::new(0, 3)]
      )]
    );
  }
}
