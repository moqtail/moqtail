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

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;

/// Stream type for a padding stream.
pub const PADDING_STREAM_TYPE: u64 = 0x132B3E28;
/// Datagram type for a padding datagram.
pub const PADDING_DATAGRAM_TYPE: u64 = 0x132B3E29;

/// A padding stream: the padding stream type followed by `length` zero bytes.
/// It carries no application data; the receiver MUST discard the contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaddingStream {
  pub length: usize,
}

impl PaddingStream {
  pub fn new(length: usize) -> Self {
    Self { length }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(PADDING_STREAM_TYPE)?;
    buf.put_bytes(0u8, self.length);
    Ok(buf.freeze())
  }

  /// Reads the leading type then consumes (discards) the remaining padding bytes.
  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let type_value = bytes.get_vi()?;
    if type_value != PADDING_STREAM_TYPE {
      return Err(ParseError::ProtocolViolation {
        context: "PaddingStream::deserialize",
        details: format!(
          "expected padding stream type {PADDING_STREAM_TYPE:#x}, got {type_value:#x}"
        ),
      });
    }
    let length = bytes.remaining();
    bytes.advance(length);
    Ok(Self { length })
  }
}

/// A padding datagram: the padding datagram type followed by `length` zero
/// bytes. It carries no application data; the receiver MUST discard it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaddingDatagram {
  pub length: usize,
}

impl PaddingDatagram {
  pub fn new(length: usize) -> Self {
    Self { length }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(PADDING_DATAGRAM_TYPE)?;
    buf.put_bytes(0u8, self.length);
    Ok(buf.freeze())
  }

  /// Reads the leading type then consumes (discards) the remaining padding bytes.
  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let type_value = bytes.get_vi()?;
    if type_value != PADDING_DATAGRAM_TYPE {
      return Err(ParseError::ProtocolViolation {
        context: "PaddingDatagram::deserialize",
        details: format!(
          "expected padding datagram type {PADDING_DATAGRAM_TYPE:#x}, got {type_value:#x}"
        ),
      });
    }
    let length = bytes.remaining();
    bytes.advance(length);
    Ok(Self { length })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_roundtrip_padding_stream() {
    let padding = PaddingStream::new(100);
    let serialized = padding.serialize().unwrap();
    // The type prefix is followed by exactly `length` bytes, all zero.
    let type_len = serialized.len() - padding.length;
    assert!(serialized.iter().skip(type_len).all(|&b| b == 0));
    let mut bytes = serialized;
    let decoded = PaddingStream::deserialize(&mut bytes).unwrap();
    assert_eq!(decoded, padding);
    assert_eq!(bytes.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_empty_padding_stream() {
    let padding = PaddingStream::new(0);
    let mut bytes = padding.serialize().unwrap();
    let decoded = PaddingStream::deserialize(&mut bytes).unwrap();
    assert_eq!(decoded, padding);
  }

  #[test]
  fn test_roundtrip_padding_datagram() {
    let padding = PaddingDatagram::new(42);
    let mut bytes = padding.serialize().unwrap();
    let decoded = PaddingDatagram::deserialize(&mut bytes).unwrap();
    assert_eq!(decoded, padding);
    assert_eq!(bytes.remaining(), 0);
  }

  #[test]
  fn test_padding_stream_wrong_type_is_error() {
    let mut bytes = PaddingDatagram::new(8).serialize().unwrap();
    assert!(matches!(
      PaddingStream::deserialize(&mut bytes),
      Err(ParseError::ProtocolViolation { .. })
    ));
  }
}
