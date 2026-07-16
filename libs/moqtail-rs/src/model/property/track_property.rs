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

use super::constant::TrackPropertyType;
use crate::model::common::pair::{
  KeyValuePair, deserialize_kvp_list_until_empty, serialize_kvp_list,
};
use crate::model::control::constant::GroupOrder;
use crate::model::error::ParseError;
use bytes::Bytes;

/// Typed representation of a MOQT track-level Property.
///
/// Unknown property types are preserved as raw KeyValuePairs so that relays
/// can forward them unchanged per spec (relays MUST NOT modify or drop unknown
/// properties).
#[derive(Debug, Clone, PartialEq)]
pub enum TrackProperty {
  ObjectDeliveryTimeout { timeout_ms: u64 },
  SubgroupDeliveryTimeout { timeout_ms: u64 },
  MaxCacheDuration { duration_ms: u64 },
  ImmutableProperties { properties: Vec<KeyValuePair> },
  DefaultPublisherPriority { priority: u8 },
  DefaultPublisherGroupOrder { order: GroupOrder },
  DynamicGroups { enabled: bool },
  Unknown { kvp: KeyValuePair },
}

impl TrackProperty {
  /// Returns the raw wire type value for this property.
  pub fn type_value(&self) -> u64 {
    match self {
      Self::ObjectDeliveryTimeout { .. } => TrackPropertyType::ObjectDeliveryTimeout as u64,
      Self::SubgroupDeliveryTimeout { .. } => TrackPropertyType::SubgroupDeliveryTimeout as u64,
      Self::MaxCacheDuration { .. } => TrackPropertyType::MaxCacheDuration as u64,
      Self::ImmutableProperties { .. } => TrackPropertyType::ImmutableProperties as u64,
      Self::DefaultPublisherPriority { .. } => TrackPropertyType::DefaultPublisherPriority as u64,
      Self::DefaultPublisherGroupOrder { .. } => {
        TrackPropertyType::DefaultPublisherGroupOrder as u64
      }
      Self::DynamicGroups { .. } => TrackPropertyType::DynamicGroups as u64,
      Self::Unknown { kvp } => kvp.get_type(),
    }
  }

  /// Serializes this property to its wire KVP bytes.
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let kvp: KeyValuePair = self.clone().try_into()?;
    kvp.serialize()
  }

  /// Deserializes a TrackProperty from a pre-parsed KeyValuePair.
  /// Unknown type values are returned as `Unknown { kvp }` rather than an error.
  pub fn deserialize(kvp: KeyValuePair) -> Result<Self, ParseError> {
    let type_value = kvp.get_type();
    let ext_type = match TrackPropertyType::try_from(type_value) {
      Ok(t) => t,
      Err(_) => return Ok(Self::Unknown { kvp }),
    };

    match ext_type {
      TrackPropertyType::ObjectDeliveryTimeout => {
        // §8: a value of 0 means no timeout is set. It is valid, not a violation.
        let value = kvp_varint_value(&kvp, "TrackProperty::deserialize(ObjectDeliveryTimeout)")?;
        Ok(Self::ObjectDeliveryTimeout { timeout_ms: value })
      }
      TrackPropertyType::SubgroupDeliveryTimeout => {
        let value = kvp_varint_value(&kvp, "TrackProperty::deserialize(SubgroupDeliveryTimeout)")?;
        Ok(Self::SubgroupDeliveryTimeout { timeout_ms: value })
      }
      TrackPropertyType::MaxCacheDuration => {
        let value = kvp_varint_value(&kvp, "TrackProperty::deserialize(MaxCacheDuration)")?;
        Ok(Self::MaxCacheDuration { duration_ms: value })
      }
      TrackPropertyType::ImmutableProperties => {
        let bytes = kvp_bytes_value(&kvp, "TrackProperty::deserialize(ImmutableProperties)")?;
        let mut buf = bytes;
        let properties = deserialize_kvp_list_until_empty(&mut buf)?;
        if properties
          .iter()
          .any(|inner| inner.get_type() == TrackPropertyType::ImmutableProperties as u64)
        {
          return Err(ParseError::ProtocolViolation {
            context: "TrackProperty::deserialize(ImmutableProperties)",
            details: "ImmutableProperties MUST NOT contain another ImmutableProperties key"
              .to_string(),
          });
        }
        Ok(Self::ImmutableProperties { properties })
      }
      TrackPropertyType::DefaultPublisherPriority => {
        let value = kvp_varint_value(&kvp, "TrackProperty::deserialize(DefaultPublisherPriority)")?;
        if value > 255 {
          return Err(ParseError::ProtocolViolation {
            context: "TrackProperty::deserialize(DefaultPublisherPriority)",
            details: format!("DEFAULT_PUBLISHER_PRIORITY must be 0-255, got {value}"),
          });
        }
        Ok(Self::DefaultPublisherPriority {
          priority: value as u8,
        })
      }
      TrackPropertyType::DefaultPublisherGroupOrder => {
        let value = kvp_varint_value(
          &kvp,
          "TrackProperty::deserialize(DefaultPublisherGroupOrder)",
        )?;
        let order = match value {
          1 => GroupOrder::Ascending,
          2 => GroupOrder::Descending,
          _ => {
            return Err(ParseError::ProtocolViolation {
              context: "TrackProperty::deserialize(DefaultPublisherGroupOrder)",
              details: format!(
                "DEFAULT_PUBLISHER_GROUP_ORDER must be 1 (Ascending) or 2 (Descending), got {value}"
              ),
            });
          }
        };
        Ok(Self::DefaultPublisherGroupOrder { order })
      }
      TrackPropertyType::DynamicGroups => {
        let value = kvp_varint_value(&kvp, "TrackProperty::deserialize(DynamicGroups)")?;
        let enabled = match value {
          0 => false,
          1 => true,
          _ => {
            return Err(ParseError::ProtocolViolation {
              context: "TrackProperty::deserialize(DynamicGroups)",
              details: format!("DYNAMIC_GROUPS must be 0 or 1, got {value}"),
            });
          }
        };
        Ok(Self::DynamicGroups { enabled })
      }
      // Object-scope properties; treated as unknown at track level
      _ => Ok(Self::Unknown { kvp }),
    }
  }
}

impl TryInto<KeyValuePair> for TrackProperty {
  type Error = ParseError;

  fn try_into(self) -> Result<KeyValuePair, Self::Error> {
    match self {
      Self::ObjectDeliveryTimeout { timeout_ms } => {
        KeyValuePair::try_new_varint(TrackPropertyType::ObjectDeliveryTimeout as u64, timeout_ms)
      }
      Self::SubgroupDeliveryTimeout { timeout_ms } => KeyValuePair::try_new_varint(
        TrackPropertyType::SubgroupDeliveryTimeout as u64,
        timeout_ms,
      ),
      Self::MaxCacheDuration { duration_ms } => {
        KeyValuePair::try_new_varint(TrackPropertyType::MaxCacheDuration as u64, duration_ms)
      }
      Self::ImmutableProperties { properties } => KeyValuePair::try_new_bytes(
        TrackPropertyType::ImmutableProperties as u64,
        serialize_kvp_list(&properties)?,
      ),
      Self::DefaultPublisherPriority { priority } => KeyValuePair::try_new_varint(
        TrackPropertyType::DefaultPublisherPriority as u64,
        priority as u64,
      ),
      Self::DefaultPublisherGroupOrder { order } => KeyValuePair::try_new_varint(
        TrackPropertyType::DefaultPublisherGroupOrder as u64,
        order as u64,
      ),
      Self::DynamicGroups { enabled } => KeyValuePair::try_new_varint(
        TrackPropertyType::DynamicGroups as u64,
        if enabled { 1 } else { 0 },
      ),
      Self::Unknown { kvp } => Ok(kvp),
    }
  }
}

/// Serializes a slice of TrackProperties to their concatenated wire KVP bytes.
/// Empty slice produces no bytes (zero wire overhead).
pub fn serialize_track_properties(exts: &[TrackProperty]) -> Result<Bytes, ParseError> {
  let kvps: Vec<KeyValuePair> = exts
    .iter()
    .map(|e| e.clone().try_into())
    .collect::<Result<_, ParseError>>()?;
  serialize_kvp_list(&kvps)
}

/// Deserializes TrackProperties from remaining payload bytes.
/// Reads KVPs until the buffer is empty. Returns an empty Vec if no bytes remain.
pub fn deserialize_track_properties(bytes: &mut Bytes) -> Result<Vec<TrackProperty>, ParseError> {
  deserialize_kvp_list_until_empty(bytes)?
    .into_iter()
    .map(TrackProperty::deserialize)
    .collect()
}

fn kvp_varint_value(kvp: &KeyValuePair, context: &'static str) -> Result<u64, ParseError> {
  match kvp {
    KeyValuePair::VarInt { value, .. } => Ok(*value),
    KeyValuePair::Bytes { type_value, .. } => Err(ParseError::ProtocolViolation {
      context,
      details: format!(
        "Property type 0x{type_value:02X} expects VarInt encoding but received Bytes"
      ),
    }),
  }
}

fn kvp_bytes_value(kvp: &KeyValuePair, context: &'static str) -> Result<Bytes, ParseError> {
  match kvp {
    KeyValuePair::Bytes { value, .. } => Ok(value.clone()),
    KeyValuePair::VarInt { type_value, .. } => Err(ParseError::ProtocolViolation {
      context,
      details: format!(
        "Property type 0x{type_value:02X} expects Bytes encoding but received VarInt"
      ),
    }),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::{Buf, Bytes};

  fn roundtrip(ext: TrackProperty) -> TrackProperty {
    let serialized = ext.serialize().unwrap();
    let mut buf = serialized;
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    assert!(!buf.has_remaining());
    TrackProperty::deserialize(kvp).unwrap()
  }

  #[test]
  fn test_roundtrip_delivery_timeout() {
    let ext = TrackProperty::ObjectDeliveryTimeout { timeout_ms: 5000 };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_roundtrip_max_cache_duration() {
    let ext = TrackProperty::MaxCacheDuration { duration_ms: 10000 };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_roundtrip_immutable_properties() {
    let inner = vec![
      KeyValuePair::try_new_varint(0x10, 42).unwrap(),
      KeyValuePair::try_new_bytes(0x11, Bytes::from_static(b"hello")).unwrap(),
    ];
    let ext = TrackProperty::ImmutableProperties {
      properties: inner.clone(),
    };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_roundtrip_default_publisher_priority() {
    let ext = TrackProperty::DefaultPublisherPriority { priority: 128 };
    assert_eq!(roundtrip(ext.clone()), ext);
    let ext_max = TrackProperty::DefaultPublisherPriority { priority: 255 };
    assert_eq!(roundtrip(ext_max.clone()), ext_max);
  }

  #[test]
  fn test_roundtrip_default_publisher_group_order() {
    for order in [GroupOrder::Ascending, GroupOrder::Descending] {
      let ext = TrackProperty::DefaultPublisherGroupOrder { order };
      assert_eq!(roundtrip(ext.clone()), ext);
    }
  }

  #[test]
  fn test_roundtrip_dynamic_groups() {
    for enabled in [true, false] {
      let ext = TrackProperty::DynamicGroups { enabled };
      assert_eq!(roundtrip(ext.clone()), ext);
    }
  }

  #[test]
  fn test_roundtrip_unknown() {
    let kvp = KeyValuePair::try_new_varint(0xFE, 99).unwrap();
    let ext = TrackProperty::Unknown { kvp };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_delivery_timeout_zero_means_no_timeout() {
    // §8: a value of 0 means no timeout is set. It is valid, not a violation.
    let kvp = KeyValuePair::try_new_varint(0x02, 0).unwrap();
    assert_eq!(
      TrackProperty::deserialize(kvp).unwrap(),
      TrackProperty::ObjectDeliveryTimeout { timeout_ms: 0 }
    );

    let kvp = KeyValuePair::try_new_varint(0x06, 0).unwrap();
    assert_eq!(
      TrackProperty::deserialize(kvp).unwrap(),
      TrackProperty::SubgroupDeliveryTimeout { timeout_ms: 0 }
    );
  }

  #[test]
  fn test_default_publisher_priority_over_255_is_error() {
    let kvp = KeyValuePair::try_new_varint(0x0E, 256).unwrap();
    assert!(matches!(
      TrackProperty::deserialize(kvp).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_default_publisher_group_order_original_is_error() {
    let kvp = KeyValuePair::try_new_varint(0x22, 0).unwrap(); // 0 = Original, invalid
    assert!(matches!(
      TrackProperty::deserialize(kvp).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_dynamic_groups_over_1_is_error() {
    let kvp = KeyValuePair::try_new_varint(0x30, 2).unwrap();
    assert!(matches!(
      TrackProperty::deserialize(kvp).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_immutable_properties_nested_is_error() {
    let inner_immutable = KeyValuePair::try_new_bytes(0x0B, Bytes::from_static(b"")).unwrap();
    let inner_bytes = inner_immutable.serialize().unwrap();
    let outer = KeyValuePair::try_new_bytes(0x0B, inner_bytes).unwrap();
    assert!(matches!(
      TrackProperty::deserialize(outer).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_serialize_deserialize_mixed() {
    let exts = vec![
      TrackProperty::ObjectDeliveryTimeout { timeout_ms: 1000 },
      TrackProperty::DefaultPublisherPriority { priority: 64 },
      TrackProperty::DynamicGroups { enabled: true },
      TrackProperty::Unknown {
        kvp: KeyValuePair::try_new_varint(0xFE, 7).unwrap(),
      },
    ];
    let serialized = serialize_track_properties(&exts).unwrap();
    let mut buf = serialized;
    let deserialized = deserialize_track_properties(&mut buf).unwrap();
    assert_eq!(deserialized, exts);
  }

  #[test]
  fn test_serialize_deserialize_empty() {
    let serialized = serialize_track_properties(&[]).unwrap();
    assert!(serialized.is_empty());
    let mut buf = serialized;
    let deserialized = deserialize_track_properties(&mut buf).unwrap();
    assert!(deserialized.is_empty());
  }

  #[test]
  fn test_immutable_properties_independent_of_outer_prev_type() {
    // A low-type property precedes ImmutableProperties (0x0B) in the outer
    // list, so the outer prev_type is nonzero by the time ImmutableProperties
    // is reached. The inner KVP list must restart its own delta state from 0,
    // independent of the outer list's running prev_type.
    let inner = vec![
      KeyValuePair::try_new_varint(0x02, 7).unwrap(),
      KeyValuePair::try_new_bytes(0x03, Bytes::from_static(b"data")).unwrap(),
    ];
    let exts = vec![
      TrackProperty::ObjectDeliveryTimeout { timeout_ms: 100 },
      TrackProperty::ImmutableProperties {
        properties: inner.clone(),
      },
    ];
    let serialized = serialize_track_properties(&exts).unwrap();
    let mut buf = serialized;
    let deserialized = deserialize_track_properties(&mut buf).unwrap();
    assert_eq!(deserialized, exts);
  }
}
