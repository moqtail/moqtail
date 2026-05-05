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
use crate::model::error::ParseError;
use bytes::{Buf, Bytes, BytesMut};

/// Typed representation of a MOQT object-level Extension Header.
///
/// Unknown extension types are preserved as raw KeyValuePairs so that relays
/// can forward them unchanged per spec.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectExtension {
  ImmutableExtensions { extensions: Vec<KeyValuePair> },
  PriorGroupIdGap { gap: u64 },
  PriorObjectIdGap { gap: u64 },
  Unknown { kvp: KeyValuePair },
}

impl ObjectExtension {
  /// Returns the raw wire type value for this extension.
  pub fn type_value(&self) -> u64 {
    match self {
      Self::ImmutableExtensions { .. } => TrackExtensionType::ImmutableExtensions as u64,
      Self::PriorGroupIdGap { .. } => TrackExtensionType::PriorGroupIdGap as u64,
      Self::PriorObjectIdGap { .. } => TrackExtensionType::PriorObjectIdGap as u64,
      Self::Unknown { kvp } => kvp.get_type(),
    }
  }

  /// Serializes this extension to its wire KVP bytes.
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let kvp: KeyValuePair = self.clone().try_into()?;
    kvp.serialize()
  }

  /// Deserializes an ObjectExtension from a pre-parsed KeyValuePair.
  /// Unknown type values are returned as `Unknown { kvp }` rather than an error.
  pub fn deserialize(kvp: KeyValuePair) -> Result<Self, ParseError> {
    let type_value = kvp.get_type();
    let ext_type = match TrackExtensionType::try_from(type_value) {
      Ok(t) => t,
      Err(_) => return Ok(Self::Unknown { kvp }),
    };

    match ext_type {
      TrackExtensionType::ImmutableExtensions => {
        let bytes = kvp_bytes_value(&kvp, "ObjectExtension::deserialize(ImmutableExtensions)")?;
        let mut buf = bytes;
        let mut extensions = Vec::new();
        while buf.has_remaining() {
          let inner = KeyValuePair::deserialize(&mut buf)?;
          if inner.get_type() == TrackExtensionType::ImmutableExtensions as u64 {
            return Err(ParseError::ProtocolViolation {
              context: "ObjectExtension::deserialize(ImmutableExtensions)",
              details: "ImmutableExtensions MUST NOT contain another ImmutableExtensions key"
                .to_string(),
            });
          }
          extensions.push(inner);
        }
        Ok(Self::ImmutableExtensions { extensions })
      }
      TrackExtensionType::PriorGroupIdGap => {
        let value = kvp_varint_value(&kvp, "ObjectExtension::deserialize(PriorGroupIdGap)")?;
        Ok(Self::PriorGroupIdGap { gap: value })
      }
      TrackExtensionType::PriorObjectIdGap => {
        let value = kvp_varint_value(&kvp, "ObjectExtension::deserialize(PriorObjectIdGap)")?;
        Ok(Self::PriorObjectIdGap { gap: value })
      }
      // Track-scope extensions; treated as unknown at object level
      _ => Ok(Self::Unknown { kvp }),
    }
  }
}

impl TryInto<KeyValuePair> for ObjectExtension {
  type Error = ParseError;

  fn try_into(self) -> Result<KeyValuePair, Self::Error> {
    match self {
      Self::ImmutableExtensions { extensions } => {
        let mut buf = BytesMut::new();
        for ext in &extensions {
          buf.extend_from_slice(&ext.serialize()?);
        }
        KeyValuePair::try_new_bytes(TrackExtensionType::ImmutableExtensions as u64, buf.freeze())
      }
      Self::PriorGroupIdGap { gap } => {
        KeyValuePair::try_new_varint(TrackExtensionType::PriorGroupIdGap as u64, gap)
      }
      Self::PriorObjectIdGap { gap } => {
        KeyValuePair::try_new_varint(TrackExtensionType::PriorObjectIdGap as u64, gap)
      }
      Self::Unknown { kvp } => Ok(kvp),
    }
  }
}

/// Serializes a slice of ObjectExtensions to their concatenated wire KVP bytes.
pub fn serialize_object_extensions(exts: &[ObjectExtension]) -> Result<Bytes, ParseError> {
  let mut buf = BytesMut::new();
  for ext in exts {
    buf.extend_from_slice(&ext.serialize()?);
  }
  Ok(buf.freeze())
}

/// Deserializes ObjectExtensions from a byte slice of known length.
/// Reads KVPs until the buffer is empty.
pub fn deserialize_object_extensions(
  bytes: &mut Bytes,
) -> Result<Vec<ObjectExtension>, ParseError> {
  let mut extensions = Vec::new();
  while bytes.has_remaining() {
    let kvp = KeyValuePair::deserialize(bytes)?;
    extensions.push(ObjectExtension::deserialize(kvp)?);
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

  fn roundtrip(ext: ObjectExtension) -> ObjectExtension {
    let serialized = ext.serialize().unwrap();
    let mut buf = serialized;
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    assert!(!buf.has_remaining());
    ObjectExtension::deserialize(kvp).unwrap()
  }

  #[test]
  fn test_roundtrip_immutable_extensions() {
    let inner = vec![
      KeyValuePair::try_new_varint(0x10, 7).unwrap(),
      KeyValuePair::try_new_bytes(0x11, Bytes::from_static(b"data")).unwrap(),
    ];
    let ext = ObjectExtension::ImmutableExtensions {
      extensions: inner.clone(),
    };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_roundtrip_prior_group_id_gap() {
    let ext = ObjectExtension::PriorGroupIdGap { gap: 3 };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_roundtrip_prior_object_id_gap() {
    let ext = ObjectExtension::PriorObjectIdGap { gap: 5 };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_roundtrip_unknown() {
    let kvp = KeyValuePair::try_new_varint(0xFE, 42).unwrap();
    let ext = ObjectExtension::Unknown { kvp };
    assert_eq!(roundtrip(ext.clone()), ext);
  }

  #[test]
  fn test_immutable_extensions_nested_is_error() {
    let inner_immutable = KeyValuePair::try_new_bytes(0x0B, Bytes::from_static(b"")).unwrap();
    let inner_bytes = inner_immutable.serialize().unwrap();
    let outer = KeyValuePair::try_new_bytes(0x0B, inner_bytes).unwrap();
    assert!(matches!(
      ObjectExtension::deserialize(outer).unwrap_err(),
      ParseError::ProtocolViolation { .. }
    ));
  }

  #[test]
  fn test_serialize_deserialize_mixed() {
    let exts = vec![
      ObjectExtension::PriorGroupIdGap { gap: 2 },
      ObjectExtension::PriorObjectIdGap { gap: 1 },
      ObjectExtension::Unknown {
        kvp: KeyValuePair::try_new_varint(0xFC, 0).unwrap(),
      },
    ];
    let serialized = serialize_object_extensions(&exts).unwrap();
    let mut buf = serialized;
    let deserialized = deserialize_object_extensions(&mut buf).unwrap();
    assert_eq!(deserialized, exts);
  }

  #[test]
  fn test_serialize_deserialize_empty() {
    let serialized = serialize_object_extensions(&[]).unwrap();
    assert!(serialized.is_empty());
    let mut buf = serialized;
    let deserialized = deserialize_object_extensions(&mut buf).unwrap();
    assert!(deserialized.is_empty());
  }
}
