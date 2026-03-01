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

use super::constant::ObjectDatagramType;

/// Draft-14 Object Datagram with payload.
///
/// Type values 0x00-0x07 encode:
/// - Bit 0: Extensions Present
/// - Bit 1: End of Group
/// - Bit 2: Object ID Absent (omitted when Object ID = 0)
#[derive(Debug, Clone, PartialEq)]
pub struct DatagramObject {
  pub track_alias: u64,
  pub group_id: u64,
  pub object_id: u64,
  pub publisher_priority: u8,
  pub extension_headers: Option<Vec<KeyValuePair>>,
  pub payload: Bytes,
  /// Draft-14: Indicates this is the last object in the group
  pub end_of_group: bool,
}

impl DatagramObject {
  /// Create a new DatagramObject without extensions (Draft-14 compliant).
  pub fn new(
    track_alias: u64,
    group_id: u64,
    object_id: u64,
    publisher_priority: u8,
    payload: Bytes,
  ) -> Self {
    DatagramObject {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers: None,
      payload,
      end_of_group: false,
    }
  }

  /// Create a new DatagramObject with all options (Draft-14 compliant).
  pub fn new_with_options(
    track_alias: u64,
    group_id: u64,
    object_id: u64,
    publisher_priority: u8,
    extension_headers: Option<Vec<KeyValuePair>>,
    payload: Bytes,
    end_of_group: bool,
  ) -> Self {
    DatagramObject {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers,
      payload,
      end_of_group,
    }
  }

  /// Create a DatagramObject with extensions.
  #[deprecated(note = "Use new_with_options() for Draft-14 compliance")]
  pub fn with_extensions(
    track_alias: u64,
    group_id: u64,
    object_id: u64,
    publisher_priority: u8,
    extension_headers: Vec<KeyValuePair>,
    payload: Bytes,
  ) -> Self {
    DatagramObject {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers: Some(extension_headers),
      payload,
      end_of_group: false,
    }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();

    // Draft-14: Determine type from properties
    let has_extensions = self.extension_headers.is_some();
    let object_id_is_zero = self.object_id == 0;
    let dtype =
      ObjectDatagramType::from_properties(has_extensions, self.end_of_group, object_id_is_zero);

    buf.put_vi(u64::from(dtype))?;
    buf.put_vi(self.track_alias)?;
    buf.put_vi(self.group_id)?;

    // Draft-14: Only write Object ID if type indicates it's present (bit 2 = 0)
    if dtype.has_object_id() {
      buf.put_vi(self.object_id)?;
    }

    buf.put_u8(self.publisher_priority);

    // Write extension headers if present
    if let Some(ext_headers) = &self.extension_headers {
      let mut payload_buf = BytesMut::new();
      for header in ext_headers {
        payload_buf.extend_from_slice(&header.serialize()?);
      }
      buf.put_vi(payload_buf.len())?;
      buf.extend_from_slice(&payload_buf);
    }

    buf.extend_from_slice(&self.payload);
    Ok(buf.freeze())
  }

  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let msg_type_raw = bytes.get_vi()?;
    let msg_type = ObjectDatagramType::try_from(msg_type_raw)?;

    let track_alias = bytes.get_vi()?;
    let group_id = bytes.get_vi()?;

    // Draft-14: Only read Object ID if type indicates it's present
    let object_id = if msg_type.has_object_id() {
      bytes.get_vi()?
    } else {
      0 // Object ID is implicitly 0 when bit 2 is set
    };

    if bytes.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "ObjectDatagram::deserialize",
        needed: 1,
        available: 0,
      });
    }

    let publisher_priority = bytes.get_u8();

    // Read extension headers if type indicates they're present
    let extension_headers = if msg_type.has_extensions() {
      let ext_len = bytes.get_vi()?;

      if ext_len == 0 {
        return Err(ParseError::ProtocolViolation {
          context: "ObjectDatagram::deserialize(extension_length)",
          details: "Extension headers present but length is 0".to_string(),
        });
      }

      let ext_len: usize =
        ext_len
          .try_into()
          .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
            context: "ObjectDatagram::deserialize",
            from_type: "u64",
            to_type: "usize",
            details: e.to_string(),
          })?;

      if bytes.remaining() < ext_len {
        return Err(ParseError::NotEnoughBytes {
          context: "ObjectDatagram::deserialize",
          needed: ext_len,
          available: bytes.remaining(),
        });
      }

      let mut header_bytes = bytes.copy_to_bytes(ext_len);
      let mut headers: Vec<KeyValuePair> = Vec::new();
      while header_bytes.has_remaining() {
        let h = KeyValuePair::deserialize(&mut header_bytes).map_err(|e| {
          ParseError::ProtocolViolation {
            context: "ObjectDatagram::deserialize, can't parse headers",
            details: e.to_string(),
          }
        })?;
        headers.push(h);
      }
      Some(headers)
    } else {
      None
    };

    let payload = bytes.copy_to_bytes(bytes.remaining());

    // Draft-14: Extract end_of_group flag from type
    let end_of_group = msg_type.is_end_of_group();

    Ok(DatagramObject {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers,
      payload,
      end_of_group,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip_with_extensions() {
    let datagram_object = DatagramObject::new_with_options(
      144,
      9,
      10,
      255,
      Some(vec![
        KeyValuePair::try_new_varint(0, 10).unwrap(),
        KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
      ]),
      Bytes::from_static(b"01239gjawkk92837aldmi"),
      false,
    );

    let mut buf = datagram_object.serialize().unwrap();
    // Type should be 0x01 (extensions, no end_of_group, object_id present)
    assert_eq!(buf[0], 0x01);

    let deserialized = DatagramObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram_object);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_without_extensions() {
    let datagram_object = DatagramObject::new(144, 9, 10, 128, Bytes::from_static(b"payload"));

    let mut buf = datagram_object.serialize().unwrap();
    // Type should be 0x00 (no extensions, no end_of_group, object_id present)
    assert_eq!(buf[0], 0x00);

    let deserialized = DatagramObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram_object);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_end_of_group() {
    let datagram_object = DatagramObject::new_with_options(
      1,
      5,
      42,
      200,
      None,
      Bytes::from_static(b"last object"),
      true, // end_of_group = true
    );

    let mut buf = datagram_object.serialize().unwrap();
    // Type should be 0x02 (no extensions, end_of_group, object_id present)
    assert_eq!(buf[0], 0x02);

    let deserialized = DatagramObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram_object);
    assert!(deserialized.end_of_group);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_object_id_zero() {
    let datagram_object = DatagramObject::new_with_options(
      1,
      5,
      0, // object_id = 0
      200,
      None,
      Bytes::from_static(b"first object"),
      false,
    );

    let mut buf = datagram_object.serialize().unwrap();
    // Type should be 0x04 (no extensions, no end_of_group, object_id absent)
    assert_eq!(buf[0], 0x04);

    let deserialized = DatagramObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram_object);
    assert_eq!(deserialized.object_id, 0);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_all_flags() {
    // extensions + end_of_group + object_id_zero = type 0x07
    let datagram_object = DatagramObject::new_with_options(
      1,
      5,
      0, // object_id = 0
      200,
      Some(vec![KeyValuePair::try_new_varint(0, 42).unwrap()]),
      Bytes::from_static(b"payload"),
      true, // end_of_group = true
    );

    let mut buf = datagram_object.serialize().unwrap();
    // Type should be 0x07 (extensions, end_of_group, object_id absent)
    assert_eq!(buf[0], 0x07);

    let deserialized = DatagramObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram_object);
    assert!(deserialized.end_of_group);
    assert_eq!(deserialized.object_id, 0);
    assert!(!buf.has_remaining());
  }
}
