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
    let mut buf = BytesMut::new();
    match self {
      Self::VarInt { type_value, value } => {
        buf.put_vi(*type_value)?;
        buf.put_vi(*value)?;
      }
      Self::Bytes { type_value, value } => {
        buf.put_vi(*type_value)?;
        buf.put_vi(value.len() as u64)?;
        buf.extend_from_slice(value);
      }
    }
    Ok(buf.freeze())
  }

  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let type_value = bytes.get_vi()?;

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
    buf.put_vi(1).unwrap(); // odd → bytes variant
    buf.put_vi(5).unwrap(); // length = 5
    buf.extend_from_slice(b"abc"); // only 3 bytes
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes).unwrap_err();
    assert!(matches!(err, ParseError::NotEnoughBytes { .. }));
  }
}
