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

use bytes::{Buf, Bytes, BytesMut};
use std::convert::TryInto;

use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;

const MAX_VALUE_LENGTH: usize = 65535; // 2^16-1

#[derive(Debug, Clone, PartialEq)]
pub enum KeyValuePair {
  VarInt { type_value: u64, value: u64 },
  Bytes { type_value: u64, value: Bytes },
}

impl KeyValuePair {
  /// Fallible constructor for a varint‐typed pair.
  pub fn try_new_varint(type_value: u64, value: u64) -> Result<Self, ParseError> {
    if !type_value.is_multiple_of(2) {
      return Err(ParseError::KeyValueFormattingError {
        context: "KeyValuePair::try_new_varint",
      });
    }
    Ok(KeyValuePair::VarInt { type_value, value })
  }

  /// Fallible constructor for a bytes‐typed pair.
  pub fn try_new_bytes(type_value: u64, value: Bytes) -> Result<Self, ParseError> {
    if type_value.is_multiple_of(2) {
      return Err(ParseError::KeyValueFormattingError {
        context: "KeyValuePair::try_new_bytes",
      });
    }
    let len = value.len();
    if len > MAX_VALUE_LENGTH {
      return Err(ParseError::LengthExceedsMax {
        context: "KeyValuePair::try_new_bytes",
        max: MAX_VALUE_LENGTH,
        len,
      });
    }
    Ok(KeyValuePair::Bytes { type_value, value })
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    self.serialize_delta(0)
  }

  /// Serializes this pair's wire form, encoding the Type as a delta from
  /// `prev_type` (the type of the previous KVP in the same list, or 0 if
  /// this is the first entry), per draft-ietf-moq-transport-16 1.4.2.
  pub fn serialize_delta(&self, prev_type: u64) -> Result<Bytes, ParseError> {
    let delta_type = self.get_type().checked_sub(prev_type).ok_or_else(|| {
      ParseError::ProtocolViolation {
        context: "KeyValuePair::serialize_delta",
        details: format!(
          "type {} is less than previous type {prev_type}; a KVP list must be sorted by ascending type to be delta-encoded",
          self.get_type()
        ),
      }
    })?;

    let mut buf = BytesMut::new();
    buf.put_vi(delta_type)?;
    match self {
      Self::VarInt { value, .. } => {
        buf.put_vi(*value)?;
      }
      Self::Bytes { value, .. } => {
        buf.put_vi(value.len() as u64)?;
        buf.extend_from_slice(value);
      }
    }
    Ok(buf.freeze())
  }

  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    Self::deserialize_delta(bytes, 0)
  }

  /// Deserializes a wire-form KVP whose Type field is a delta from
  /// `prev_type` (the type of the previous KVP in the same list, or 0 if
  /// this is the first entry), per draft-ietf-moq-transport-16 1.4.2.
  pub fn deserialize_delta(bytes: &mut Bytes, prev_type: u64) -> Result<Self, ParseError> {
    let delta_type = bytes.get_vi()?;
    let type_value =
      prev_type
        .checked_add(delta_type)
        .ok_or_else(|| ParseError::ProtocolViolation {
          context: "KeyValuePair::deserialize_delta",
          details: format!(
            "previous type {prev_type} plus delta type {delta_type} exceeds 2^64 - 1"
          ),
        })?;

    if type_value % 2 == 0 {
      // VarInt variant
      let value = bytes.get_vi()?;
      Ok(KeyValuePair::VarInt { type_value, value })
    } else {
      // Bytes variant
      let len_u64 = bytes.get_vi()?;
      let len: usize =
        len_u64
          .try_into()
          .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
            context: "KeyValuePair::deserialize length",
            from_type: "u64",
            to_type: "usize",
            details: e.to_string(),
          })?;

      if len > MAX_VALUE_LENGTH {
        return Err(ParseError::LengthExceedsMax {
          context: "KeyValuePair::deserialize",
          max: MAX_VALUE_LENGTH,
          len,
        });
      }
      if bytes.remaining() < len {
        return Err(ParseError::NotEnoughBytes {
          context: "KeyValuePair::deserialize value",
          needed: len,
          available: bytes.remaining(),
        });
      }

      let value = bytes.copy_to_bytes(len);

      Ok(KeyValuePair::Bytes { type_value, value })
    }
  }

  pub fn is_same_type(&self, other: &KeyValuePair) -> bool {
    self.get_type() == other.get_type()
  }

  pub fn get_type(&self) -> u64 {
    match self {
      KeyValuePair::VarInt { type_value, .. } => *type_value,
      KeyValuePair::Bytes { type_value, .. } => *type_value,
    }
  }
}

/// Delta-encodes and serializes a list of Key-Value-Pairs to wire bytes.
/// The list is sorted by ascending type first, since Delta Type is an
/// unsigned varint and cannot represent a type decrease.
pub fn serialize_kvp_list(items: &[KeyValuePair]) -> Result<Bytes, ParseError> {
  let mut sorted: Vec<&KeyValuePair> = items.iter().collect();
  sorted.sort_by_key(|kvp| kvp.get_type());

  let mut buf = BytesMut::new();
  let mut prev_type = 0u64;
  for kvp in sorted {
    buf.extend_from_slice(&kvp.serialize_delta(prev_type)?);
    prev_type = kvp.get_type();
  }
  Ok(buf.freeze())
}

/// Reads exactly `count` delta-encoded Key-Value-Pairs from `bytes`.
pub fn deserialize_kvp_list(
  bytes: &mut Bytes,
  count: u64,
) -> Result<Vec<KeyValuePair>, ParseError> {
  let mut items = Vec::with_capacity(count as usize);
  let mut prev_type = 0u64;
  for _ in 0..count {
    let kvp = KeyValuePair::deserialize_delta(bytes, prev_type)?;
    prev_type = kvp.get_type();
    items.push(kvp);
  }
  Ok(items)
}

/// Reads delta-encoded Key-Value-Pairs from `bytes` until it is exhausted.
pub fn deserialize_kvp_list_until_empty(
  bytes: &mut Bytes,
) -> Result<Vec<KeyValuePair>, ParseError> {
  let mut items = Vec::new();
  let mut prev_type = 0u64;
  while bytes.has_remaining() {
    let kvp = KeyValuePair::deserialize_delta(bytes, prev_type)?;
    prev_type = kvp.get_type();
    items.push(kvp);
  }
  Ok(items)
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::{Bytes, BytesMut};

  #[test]

  fn roundtrip_varint() {
    let original = KeyValuePair::try_new_varint(2, 100).unwrap();
    let mut buf = original.serialize().unwrap();
    let parsed = KeyValuePair::deserialize(&mut buf).unwrap();
    assert_eq!(parsed, original);
  }

  #[test]
  fn roundtrip_bytes() {
    let original = KeyValuePair::try_new_bytes(1, Bytes::from("test")).unwrap();
    let mut buf = original.serialize().unwrap();
    let parsed = KeyValuePair::deserialize(&mut buf).unwrap();
    assert_eq!(parsed, original);
  }

  #[test]
  fn invalid_type_varint() {
    let err = KeyValuePair::try_new_varint(1, 100);
    assert!(err.is_err());
  }

  #[test]
  fn invalid_type_bytes() {
    let err = KeyValuePair::try_new_bytes(2, Bytes::from("x"));
    assert!(err.is_err());
  }

  #[test]
  fn length_exceeds_max() {
    let data = Bytes::from(vec![0u8; MAX_VALUE_LENGTH + 1]);
    let err = KeyValuePair::try_new_bytes(1, data).unwrap_err();
    assert!(matches!(err, ParseError::LengthExceedsMax { .. }));
  }

  #[test]
  fn deserialize_not_enough_bytes() {
    let mut buf = BytesMut::new();
    buf.put_vi(1).unwrap(); // odd -> bytes variant
    buf.put_vi(5).unwrap(); // length = 5
    buf.extend_from_slice(b"abc"); // only 3 bytes
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes).unwrap_err();
    assert!(matches!(err, ParseError::NotEnoughBytes { .. }));
  }

  #[test]
  fn is_same_type_test() {
    let kv1 = KeyValuePair::try_new_varint(2, 100).unwrap();
    let kv2 = KeyValuePair::try_new_varint(2, 200).unwrap();
    let kv3 = KeyValuePair::try_new_bytes(1, Bytes::from("test")).unwrap();
    assert!(kv1.is_same_type(&kv2));
    assert!(!kv1.is_same_type(&kv3));
  }

  #[test]
  fn get_type_test() {
    let kv1 = KeyValuePair::try_new_varint(2, 100).unwrap();
    let kv2 = KeyValuePair::try_new_bytes(1, Bytes::from("test")).unwrap();
    assert_eq!(kv1.get_type(), 2);
    assert_eq!(kv2.get_type(), 1);
  }

  #[test]
  fn deserialize_delta_overflow_is_protocol_violation() {
    // previous_type + delta_type must not exceed u64::MAX per draft-16 1.4.2.
    let mut buf = BytesMut::new();
    buf.put_vi(10u64).unwrap();
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize_delta(&mut bytes, u64::MAX - 5).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn serialize_delta_decreasing_type_is_protocol_violation() {
    // Delta Type is an unsigned varint, so a type lower than prev_type can't be encoded.
    let kvp = KeyValuePair::try_new_varint(2, 1).unwrap();
    let err = kvp.serialize_delta(10).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn serialize_kvp_list_sorts_and_delta_encodes() {
    let items = vec![
      KeyValuePair::try_new_varint(0x20, 1).unwrap(),
      KeyValuePair::try_new_varint(0x10, 2).unwrap(),
      KeyValuePair::try_new_bytes(0x21, Bytes::from_static(b"x")).unwrap(),
    ];
    let mut bytes = serialize_kvp_list(&items).unwrap();
    let decoded = deserialize_kvp_list(&mut bytes, 3).unwrap();
    let types: Vec<u64> = decoded.iter().map(|k| k.get_type()).collect();
    assert_eq!(types, vec![0x10, 0x20, 0x21]);
  }

  #[test]
  fn deserialize_kvp_list_until_empty_reads_delta_encoded_stream() {
    let items = vec![
      KeyValuePair::try_new_varint(0x04, 1).unwrap(),
      KeyValuePair::try_new_varint(0x02, 2).unwrap(),
    ];
    let mut bytes = serialize_kvp_list(&items).unwrap();
    let decoded = deserialize_kvp_list_until_empty(&mut bytes).unwrap();
    let types: Vec<u64> = decoded.iter().map(|k| k.get_type()).collect();
    assert_eq!(types, vec![0x02, 0x04]);
  }
}
