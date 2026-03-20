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

use super::constant::TrackExtensionType;
use crate::model::common::pair::KeyValuePair;
use crate::model::control::constant::GroupOrder;
use crate::model::error::ParseError;
use bytes::{Buf, Bytes, BytesMut};

/// Typed representation of a MOQT track-level Extension Header.
///
/// Unknown extension types are preserved as raw KeyValuePairs so that relays
/// can forward them unchanged per spec (relays MUST NOT modify or drop unknown
/// extensions).
#[derive(Debug, Clone, PartialEq)]
pub enum TrackExtension {
  DeliveryTimeout { timeout_ms: u64 },
  MaxCacheDuration { duration_ms: u64 },
  ImmutableExtensions { extensions: Vec<KeyValuePair> },
  DefaultPublisherPriority { priority: u8 },
  DefaultPublisherGroupOrder { order: GroupOrder },
  DynamicGroups { enabled: bool },
  Unknown { kvp: KeyValuePair },
}

impl TrackExtension {
  /// Returns the raw wire type value for this extension.
  pub fn type_value(&self) -> u64 {
    match self {
      Self::DeliveryTimeout { .. } => TrackExtensionType::DeliveryTimeout as u64,
      Self::MaxCacheDuration { .. } => TrackExtensionType::MaxCacheDuration as u64,
      Self::ImmutableExtensions { .. } => TrackExtensionType::ImmutableExtensions as u64,
      Self::DefaultPublisherPriority { .. } => TrackExtensionType::DefaultPublisherPriority as u64,
      Self::DefaultPublisherGroupOrder { .. } => {
        TrackExtensionType::DefaultPublisherGroupOrder as u64
      }
      Self::DynamicGroups { .. } => TrackExtensionType::DynamicGroups as u64,
      Self::Unknown { kvp } => kvp.get_type(),
    }
  }

  /// Serializes this extension to its wire KVP bytes.
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let kvp: KeyValuePair = self.clone().try_into()?;
    kvp.serialize()
  }

  /// Deserializes a TrackExtension from a pre-parsed KeyValuePair.
  /// Unknown type values are returned as `Unknown { kvp }` rather than an error.
  pub fn deserialize(kvp: KeyValuePair) -> Result<Self, ParseError> {
    let type_value = kvp.get_type();
    let ext_type = match TrackExtensionType::try_from(type_value) {
      Ok(t) => t,
      Err(_) => return Ok(Self::Unknown { kvp }),
    };

    match ext_type {
      TrackExtensionType::DeliveryTimeout => {
        let value = kvp_varint_value(&kvp, "TrackExtension::deserialize(DeliveryTimeout)")?;
        if value == 0 {
          return Err(ParseError::ProtocolViolation {
            context: "TrackExtension::deserialize(DeliveryTimeout)",
            details: "DELIVERY_TIMEOUT must be greater than 0".to_string(),
          });
        }
        Ok(Self::DeliveryTimeout { timeout_ms: value })
      }
      TrackExtensionType::MaxCacheDuration => {
        let value = kvp_varint_value(&kvp, "TrackExtension::deserialize(MaxCacheDuration)")?;
        Ok(Self::MaxCacheDuration { duration_ms: value })
      }
      TrackExtensionType::ImmutableExtensions => {
        let bytes = kvp_bytes_value(&kvp, "TrackExtension::deserialize(ImmutableExtensions)")?;
        let mut buf = bytes;
        let mut extensions = Vec::new();
        while buf.has_remaining() {
          let inner = KeyValuePair::deserialize(&mut buf)?;
          if inner.get_type() == TrackExtensionType::ImmutableExtensions as u64 {
            return Err(ParseError::ProtocolViolation {
              context: "TrackExtension::deserialize(ImmutableExtensions)",
              details: "ImmutableExtensions MUST NOT contain another ImmutableExtensions key"
                .to_string(),
            });
          }
          extensions.push(inner);
        }
        Ok(Self::ImmutableExtensions { extensions })
      }
      TrackExtensionType::DefaultPublisherPriority => {
        let value = kvp_varint_value(
          &kvp,
          "TrackExtension::deserialize(DefaultPublisherPriority)",
        )?;
        if value > 255 {
          return Err(ParseError::ProtocolViolation {
            context: "TrackExtension::deserialize(DefaultPublisherPriority)",
            details: format!("DEFAULT_PUBLISHER_PRIORITY must be 0-255, got {value}"),
          });
        }
        Ok(Self::DefaultPublisherPriority {
          priority: value as u8,
        })
      }
      TrackExtensionType::DefaultPublisherGroupOrder => {
        let value = kvp_varint_value(
          &kvp,
          "TrackExtension::deserialize(DefaultPublisherGroupOrder)",
        )?;
        let order = match value {
          1 => GroupOrder::Ascending,
          2 => GroupOrder::Descending,
          _ => {
            return Err(ParseError::ProtocolViolation {
              context: "TrackExtension::deserialize(DefaultPublisherGroupOrder)",
              details: format!(
                "DEFAULT_PUBLISHER_GROUP_ORDER must be 1 (Ascending) or 2 (Descending), got {value}"
              ),
            });
          }
        };
        Ok(Self::DefaultPublisherGroupOrder { order })
      }
      TrackExtensionType::DynamicGroups => {
        let value = kvp_varint_value(&kvp, "TrackExtension::deserialize(DynamicGroups)")?;
        let enabled = match value {
          0 => false,
          1 => true,
          _ => {
            return Err(ParseError::ProtocolViolation {
              context: "TrackExtension::deserialize(DynamicGroups)",
              details: format!("DYNAMIC_GROUPS must be 0 or 1, got {value}"),
            });
          }
        };
        Ok(Self::DynamicGroups { enabled })
      }
      // Object-scope extensions; treated as unknown at track level
      _ => Ok(Self::Unknown { kvp }),
    }
  }
}

impl TryInto<KeyValuePair> for TrackExtension {
  type Error = ParseError;

  fn try_into(self) -> Result<KeyValuePair, Self::Error> {
    match self {
      Self::DeliveryTimeout { timeout_ms } => {
        KeyValuePair::try_new_varint(TrackExtensionType::DeliveryTimeout as u64, timeout_ms)
      }
      Self::MaxCacheDuration { duration_ms } => {
        KeyValuePair::try_new_varint(TrackExtensionType::MaxCacheDuration as u64, duration_ms)
      }
      Self::ImmutableExtensions { extensions } => {
        let mut buf = BytesMut::new();
        for ext in &extensions {
          buf.extend_from_slice(&ext.serialize()?);
        }
        KeyValuePair::try_new_bytes(TrackExtensionType::ImmutableExtensions as u64, buf.freeze())
      }
      Self::DefaultPublisherPriority { priority } => KeyValuePair::try_new_varint(
        TrackExtensionType::DefaultPublisherPriority as u64,
        priority as u64,
      ),
      Self::DefaultPublisherGroupOrder { order } => KeyValuePair::try_new_varint(
        TrackExtensionType::DefaultPublisherGroupOrder as u64,
        order as u64,
      ),
      Self::DynamicGroups { enabled } => KeyValuePair::try_new_varint(
        TrackExtensionType::DynamicGroups as u64,
        if enabled { 1 } else { 0 },
      ),
      Self::Unknown { kvp } => Ok(kvp),
    }
  }
}

/// Serializes a slice of TrackExtensions to their concatenated wire KVP bytes.
/// Empty slice produces no bytes (zero wire overhead).
pub fn serialize_track_extensions(exts: &[TrackExtension]) -> Result<Bytes, ParseError> {
  let mut buf = BytesMut::new();
  for ext in exts {
    buf.extend_from_slice(&ext.serialize()?);
  }
  Ok(buf.freeze())
}

/// Deserializes TrackExtensions from remaining payload bytes.
/// Reads KVPs until the buffer is empty. Returns an empty Vec if no bytes remain.
pub fn deserialize_track_extensions(bytes: &mut Bytes) -> Result<Vec<TrackExtension>, ParseError> {
  let mut extensions = Vec::new();
  while bytes.has_remaining() {
    let kvp = KeyValuePair::deserialize(bytes)?;
    extensions.push(TrackExtension::deserialize(kvp)?);
  }
  Ok(extensions)
}

fn kvp_varint_value(kvp: &KeyValuePair, context: &'static str) -> Result<u64, ParseError> {
  match kvp {
    KeyValuePair::VarInt { value, .. } => Ok(*value),
    KeyValuePair::Bytes { type_value, .. } => Err(ParseError::ProtocolViolation {
      context,
      details: format!(
        "Extension type 0x{type_value:02X} expects VarInt encoding but received Bytes"
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
        "Extension type 0x{type_value:02X} expects Bytes encoding but received VarInt"
      ),
    }),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Bytes;

  fn roundtrip(ext: TrackExtension) -> TrackExtension {
    let serialized = ext.serialize().unwrap();
    let mut buf = serialized;
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    assert!(!buf.has_remaining());
    TrackExtension::deserialize(kvp).unwrap()
  }

  #[test]
  fn test_roundtrip_delivery_timeout() {
    let ext = TrackExtension::DeliveryTimeout { timeout_ms: 5000 };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_roundtrip_max_cache_duration() {
    let ext = TrackExtension::MaxCacheDuration { duration_ms: 10000 };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_roundtrip_immutable_extensions() {
    let inner = vec![
      KeyValuePair::try_new_varint(0x10, 42).unwrap(),
      KeyValuePair::try_new_bytes(0x11, Bytes::from_static(b"hello")).unwrap(),
    ];
    let ext = TrackExtension::ImmutableExtensions {
      extensions: inner.clone(),
    };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_roundtrip_default_publisher_priority() {
    let ext = TrackExtension::DefaultPublisherPriority { priority: 128 };
    assert_eq!(roundtrip(ext.clone()), ext);
    let ext_max = TrackExtension::DefaultPublisherPriority { priority: 255 };
    assert_eq!(roundtrip(ext_max.clone()), ext_max);
  }

  #[test]
  fn test_roundtrip_default_publisher_group_order() {
    for order in [GroupOrder::Ascending, GroupOrder::Descending] {
      let ext = TrackExtension::DefaultPublisherGroupOrder { order };
      assert_eq!(roundtrip(ext.clone()), ext);
    }
  }

  #[test]
  fn test_roundtrip_dynamic_groups() {
    for enabled in [true, false] {
      let ext = TrackExtension::DynamicGroups { enabled };
      assert_eq!(roundtrip(ext.clone()), ext);
    }
  }

  #[test]
  fn test_roundtrip_unknown() {
    let kvp = KeyValuePair::try_new_varint(0xFE, 99).unwrap();
    let ext = TrackExtension::Unknown { kvp };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_delivery_timeout_zero_is_error() {
    let kvp = KeyValuePair::try_new_varint(0x02, 0).unwrap();
    assert!(matches!(
      TrackExtension::deserialize(kvp).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_default_publisher_priority_over_255_is_error() {
    let kvp = KeyValuePair::try_new_varint(0x0E, 256).unwrap();
    assert!(matches!(
      TrackExtension::deserialize(kvp).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_default_publisher_group_order_original_is_error() {
    let kvp = KeyValuePair::try_new_varint(0x22, 0).unwrap(); // 0 = Original, invalid
    assert!(matches!(
      TrackExtension::deserialize(kvp).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_dynamic_groups_over_1_is_error() {
    let kvp = KeyValuePair::try_new_varint(0x30, 2).unwrap();
    assert!(matches!(
      TrackExtension::deserialize(kvp).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_immutable_extensions_nested_is_error() {
    let inner_immutable = KeyValuePair::try_new_bytes(0x0B, Bytes::from_static(b"")).unwrap();
    let inner_bytes = inner_immutable.serialize().unwrap();
    let outer = KeyValuePair::try_new_bytes(0x0B, inner_bytes).unwrap();
    assert!(matches!(
      TrackExtension::deserialize(outer).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_serialize_deserialize_mixed() {
    let exts = vec![
      TrackExtension::DeliveryTimeout { timeout_ms: 1000 },
      TrackExtension::DefaultPublisherPriority { priority: 64 },
      TrackExtension::DynamicGroups { enabled: true },
      TrackExtension::Unknown {
        kvp: KeyValuePair::try_new_varint(0xFE, 7).unwrap(),
      },
    ];
    let serialized = serialize_track_extensions(&exts).unwrap();
    let mut buf = serialized;
    let deserialized = deserialize_track_extensions(&mut buf).unwrap();
    assert_eq!(deserialized, exts);
  }

  #[test]
  fn test_serialize_deserialize_empty() {
    let serialized = serialize_track_extensions(&[]).unwrap();
    assert!(serialized.is_empty());
    let mut buf = serialized;
    let deserialized = deserialize_track_extensions(&mut buf).unwrap();
    assert!(deserialized.is_empty());
  }
}
