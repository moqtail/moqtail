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

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;

use super::constant::{ObjectDatagramStatusType, ObjectStatus};

/// Draft-14 Object Datagram Status (no payload, only status).
///
/// Type values:
/// - 0x20: Without Extensions
/// - 0x21: With Extensions
///
/// Object ID is always present in status datagrams.
#[derive(Debug, Clone, PartialEq)]
pub struct DatagramStatus {
  pub track_alias: u64,
  pub group_id: u64,
  pub object_id: u64,
  pub publisher_priority: u8,
  pub extension_headers: Option<Vec<KeyValuePair>>,
  pub object_status: ObjectStatus,
}

impl DatagramStatus {
  pub fn new(
    track_alias: u64,
    group_id: u64,
    object_id: u64,
    publisher_priority: u8,
    object_status: ObjectStatus,
  ) -> Self {
    DatagramStatus {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers: None,
      object_status,
    }
  }

  pub fn with_extensions(
    track_alias: u64,
    group_id: u64,
    object_id: u64,
    publisher_priority: u8,
    extension_headers: Vec<KeyValuePair>,
    object_status: ObjectStatus,
  ) -> Self {
    DatagramStatus {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers: Some(extension_headers),
      object_status,
    }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();

    // Draft-14: Type 0x20 if no extensions, 0x21 if extensions present
    let dtype = if self.extension_headers.is_some() {
      ObjectDatagramStatusType::WithExtensions
    } else {
      ObjectDatagramStatusType::WithoutExtensions
    };

    buf.put_vi(u64::from(dtype))?;
    buf.put_vi(self.track_alias)?;
    buf.put_vi(self.group_id)?;
    buf.put_vi(self.object_id)?; // Object ID always present in status datagrams
    buf.put_u8(self.publisher_priority);

    // Write extension headers if present
    if let Some(ext_headers) = &self.extension_headers {
      let mut payload = BytesMut::new();
      for header in ext_headers {
        payload.extend_from_slice(&header.serialize()?);
      }
      buf.put_vi(payload.len())?;
      buf.extend_from_slice(&payload);
    }

    buf.put_vi(self.object_status)?;
    Ok(buf.freeze())
  }

  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let msg_type_raw = bytes.get_vi()?;
    let msg_type = ObjectDatagramStatusType::try_from(msg_type_raw)?;

    let track_alias = bytes.get_vi()?;
    let group_id = bytes.get_vi()?;
    let object_id = bytes.get_vi()?; // Object ID always present in status datagrams

    if bytes.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "ObjectDatagramStatus::deserialize",
        needed: 1,
        available: 0,
      });
    }

    let publisher_priority = bytes.get_u8();

    let extension_headers = if msg_type.has_extensions() {
      let ext_len = bytes.get_vi()?;

      if ext_len == 0 {
        return Err(ParseError::ProtocolViolation {
          context: "ObjectDatagramStatus::deserialize(ext_len)",
          details: "Extension headers present (Type=0x21) but length is 0".to_string(),
        });
      }
      let ext_len: usize =
        ext_len
          .try_into()
          .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
            context: "ObjectDatagramStatus::deserialize",
            from_type: "u64",
            to_type: "usize",
            details: e.to_string(),
          })?;

      if bytes.remaining() < ext_len {
        return Err(ParseError::NotEnoughBytes {
          context: "ObjectDatagramStatus::deserialize",
          needed: ext_len,
          available: bytes.remaining(),
        });
      }
      let mut header_bytes = bytes.copy_to_bytes(ext_len);
      let mut headers: Vec<KeyValuePair> = Vec::new();
      while header_bytes.has_remaining() {
        let h = KeyValuePair::deserialize(&mut header_bytes).map_err(|_| {
          ParseError::ProtocolViolation {
            context: "ObjectDatagramStatus::deserialize(headers)",
            details: "Should be able to parse headers".to_string(),
          }
        })?;
        headers.push(h);
      }
      Some(headers)
    } else {
      None
    };

    let object_status_raw = bytes.get_vi()?;
    let object_status = ObjectStatus::try_from(object_status_raw)?;

    Ok(DatagramStatus {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers,
      object_status,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip_with_extensions() {
    let datagram_status = DatagramStatus::with_extensions(
      144,
      9,
      10,
      255,
      vec![
        KeyValuePair::try_new_varint(0, 10).unwrap(),
        KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
      ],
      ObjectStatus::Normal,
    );

    let mut buf = datagram_status.serialize().unwrap();
    // Type should be 0x21 (with extensions)
    assert_eq!(buf[0], 0x21);

    let deserialized = DatagramStatus::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram_status);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_without_extensions() {
    let datagram_status = DatagramStatus::new(144, 9, 10, 128, ObjectStatus::EndOfGroup);

    let mut buf = datagram_status.serialize().unwrap();
    // Type should be 0x20 (without extensions)
    assert_eq!(buf[0], 0x20);

    let deserialized = DatagramStatus::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram_status);
    assert!(!buf.has_remaining());
  }
}
