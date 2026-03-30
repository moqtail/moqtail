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

use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;

use super::constant::{ObjectDatagramType, ObjectStatus};

/// Draft-16 unified Object Datagram.
///
/// Type bit layout (form 0b00X0XXXX):
/// - Bit 0 (0x01): EXTENSIONS - Extensions field present
/// - Bit 1 (0x02): END_OF_GROUP - Last object in group
/// - Bit 2 (0x04): ZERO_OBJECT_ID - Object ID omitted (assumed 0)
/// - Bit 3 (0x08): DEFAULT_PRIORITY - Publisher Priority omitted (inherited)
/// - Bit 5 (0x20): STATUS - Object Status replaces Object Payload
///
/// Payload datagrams carry `payload` (Some) and `object_status` (None).
/// Status datagrams carry `object_status` (Some) and `payload` (None).
#[derive(Debug, Clone, PartialEq)]
pub struct Datagram {
  pub track_alias: u64,
  pub group_id: u64,
  pub object_id: u64,
  pub publisher_priority: Option<u8>,
  pub extension_headers: Option<Vec<KeyValuePair>>,
  pub payload: Option<Bytes>,
  pub object_status: Option<ObjectStatus>,
  pub end_of_group: bool,
}

impl Datagram {
  /// Create a new payload datagram.
  pub fn new_payload(
    track_alias: u64,
    group_id: u64,
    object_id: u64,
    publisher_priority: Option<u8>,
    extension_headers: Option<Vec<KeyValuePair>>,
    payload: Bytes,
    end_of_group: bool,
  ) -> Self {
    Datagram {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers,
      payload: Some(payload),
      object_status: None,
      end_of_group,
    }
  }

  /// Create a new status datagram.
  /// Note: end_of_group is forced false (STATUS + END_OF_GROUP is a PROTOCOL_VIOLATION).
  pub fn new_status(
    track_alias: u64,
    group_id: u64,
    object_id: u64,
    publisher_priority: Option<u8>,
    extension_headers: Option<Vec<KeyValuePair>>,
    object_status: ObjectStatus,
  ) -> Self {
    Datagram {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers,
      payload: None,
      object_status: Some(object_status),
      end_of_group: false,
    }
  }

  /// Create a simple payload datagram without extensions.
  pub fn new(
    track_alias: u64,
    group_id: u64,
    object_id: u64,
    publisher_priority: u8,
    payload: Bytes,
  ) -> Self {
    Datagram {
      track_alias,
      group_id,
      object_id,
      publisher_priority: Some(publisher_priority),
      extension_headers: None,
      payload: Some(payload),
      object_status: None,
      end_of_group: false,
    }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();

    let has_extensions = self.extension_headers.is_some();
    let object_id_is_zero = self.object_id == 0;
    let default_priority = self.publisher_priority.is_none();
    let is_status = self.object_status.is_some();

    let dtype = ObjectDatagramType::from_properties(
      has_extensions,
      self.end_of_group,
      object_id_is_zero,
      default_priority,
      is_status,
    )?;

    buf.put_vi(u64::from(dtype))?;
    buf.put_vi(self.track_alias)?;
    buf.put_vi(self.group_id)?;

    // Only write Object ID if ZERO_OBJECT_ID bit is not set
    if !dtype.is_zero_object_id() {
      buf.put_vi(self.object_id)?;
    }

    // Only write Publisher Priority if DEFAULT_PRIORITY bit is not set
    if !dtype.has_default_priority() {
      let priority = self.publisher_priority.unwrap_or(0);
      buf.put_u8(priority);
    }

    // Write extension headers if present
    if let Some(ext_headers) = &self.extension_headers {
      let mut payload_buf = BytesMut::new();
      for header in ext_headers {
        payload_buf.extend_from_slice(&header.serialize()?);
      }
      buf.put_vi(payload_buf.len())?;
      buf.extend_from_slice(&payload_buf);
    }

    // Write payload or object status
    if is_status {
      let status = self.object_status.unwrap();
      buf.put_vi(status)?;
    } else if let Some(payload) = &self.payload {
      buf.extend_from_slice(payload);
    }

    Ok(buf.freeze())
  }

  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let msg_type_raw = bytes.get_vi()?;
    let msg_type = ObjectDatagramType::try_from(msg_type_raw)?;

    let track_alias = bytes.get_vi()?;
    let group_id = bytes.get_vi()?;

    // Only read Object ID if ZERO_OBJECT_ID bit is not set
    let object_id = if !msg_type.is_zero_object_id() {
      bytes.get_vi()?
    } else {
      0
    };

    // Only read Publisher Priority if DEFAULT_PRIORITY bit is not set
    let publisher_priority = if !msg_type.has_default_priority() {
      if bytes.remaining() < 1 {
        return Err(ParseError::NotEnoughBytes {
          context: "Datagram::deserialize(publisher_priority)",
          needed: 1,
          available: 0,
        });
      }
      Some(bytes.get_u8())
    } else {
      None
    };

    // Read extension headers if type indicates they're present
    let extension_headers = if msg_type.has_extensions() {
      let ext_len = bytes.get_vi()?;

      if ext_len == 0 {
        return Err(ParseError::ProtocolViolation {
          context: "Datagram::deserialize(extension_length)",
          details: "Extension headers present but length is 0".to_string(),
        });
      }

      let ext_len: usize =
        ext_len
          .try_into()
          .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
            context: "Datagram::deserialize",
            from_type: "u64",
            to_type: "usize",
            details: e.to_string(),
          })?;

      if bytes.remaining() < ext_len {
        return Err(ParseError::NotEnoughBytes {
          context: "Datagram::deserialize(extension_headers)",
          needed: ext_len,
          available: bytes.remaining(),
        });
      }

      let mut header_bytes = bytes.copy_to_bytes(ext_len);
      let mut headers: Vec<KeyValuePair> = Vec::new();
      while header_bytes.has_remaining() {
        let h = KeyValuePair::deserialize(&mut header_bytes).map_err(|e| {
          ParseError::ProtocolViolation {
            context: "Datagram::deserialize, can't parse headers",
            details: e.to_string(),
          }
        })?;
        headers.push(h);
      }
      Some(headers)
    } else {
      None
    };

    // Read payload or object status based on STATUS bit
    let (payload, object_status) = if msg_type.is_status() {
      let status_raw = bytes.get_vi()?;
      let status = ObjectStatus::try_from(status_raw)?;
      (None, Some(status))
    } else {
      let payload = bytes.copy_to_bytes(bytes.remaining());
      (Some(payload), None)
    };

    let end_of_group = msg_type.is_end_of_group();

    Ok(Datagram {
      track_alias,
      group_id,
      object_id,
      publisher_priority,
      extension_headers,
      payload,
      object_status,
      end_of_group,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip_payload_no_flags() {
    let datagram = Datagram::new_payload(144, 9, 10, Some(128), None, Bytes::from_static(b"payload"), false);

    let mut buf = datagram.serialize().unwrap();
    // Type 0x00: no extensions, no end_of_group, object_id present, priority present, not status
    assert_eq!(buf[0], 0x00);

    let deserialized = Datagram::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_payload_with_extensions() {
    let datagram = Datagram::new_payload(
      144,
      9,
      10,
      Some(255),
      Some(vec![
        KeyValuePair::try_new_varint(0, 10).unwrap(),
        KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
      ]),
      Bytes::from_static(b"01239gjawkk92837aldmi"),
      false,
    );

    let mut buf = datagram.serialize().unwrap();
    // Type 0x01: extensions present
    assert_eq!(buf[0], 0x01);

    let deserialized = Datagram::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_end_of_group() {
    let datagram = Datagram::new_payload(
      1,
      5,
      42,
      Some(200),
      None,
      Bytes::from_static(b"last object"),
      true,
    );

    let mut buf = datagram.serialize().unwrap();
    // Type 0x02: end_of_group set
    assert_eq!(buf[0], 0x02);

    let deserialized = Datagram::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram);
    assert!(deserialized.end_of_group);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_zero_object_id() {
    let datagram = Datagram::new_payload(
      1,
      5,
      0,
      Some(200),
      None,
      Bytes::from_static(b"first object"),
      false,
    );

    let mut buf = datagram.serialize().unwrap();
    // Type 0x04: ZERO_OBJECT_ID set
    assert_eq!(buf[0], 0x04);

    let deserialized = Datagram::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram);
    assert_eq!(deserialized.object_id, 0);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_default_priority() {
    let datagram = Datagram::new_payload(
      1,
      5,
      10,
      None, // DEFAULT_PRIORITY
      None,
      Bytes::from_static(b"data"),
      false,
    );

    let mut buf = datagram.serialize().unwrap();
    // Type 0x08: DEFAULT_PRIORITY set
    assert_eq!(buf[0], 0x08);

    let deserialized = Datagram::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram);
    assert!(deserialized.publisher_priority.is_none());
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_all_payload_flags() {
    // extensions + end_of_group + zero_object_id + default_priority = 0x0F
    let datagram = Datagram::new_payload(
      1,
      5,
      0,
      None,
      Some(vec![KeyValuePair::try_new_varint(0, 42).unwrap()]),
      Bytes::from_static(b"payload"),
      true,
    );

    let mut buf = datagram.serialize().unwrap();
    // Type 0x0F: all payload bits set
    assert_eq!(buf[0], 0x0F);

    let deserialized = Datagram::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram);
    assert!(deserialized.end_of_group);
    assert_eq!(deserialized.object_id, 0);
    assert!(deserialized.publisher_priority.is_none());
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_status_without_extensions() {
    let datagram = Datagram::new_status(144, 9, 10, Some(128), None, ObjectStatus::EndOfGroup);

    let mut buf = datagram.serialize().unwrap();
    // Type 0x20: STATUS set, no extensions
    assert_eq!(buf[0], 0x20);

    let deserialized = Datagram::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_status_with_extensions() {
    let datagram = Datagram::new_status(
      144,
      9,
      10,
      Some(255),
      Some(vec![
        KeyValuePair::try_new_varint(0, 10).unwrap(),
        KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
      ]),
      ObjectStatus::Normal,
    );

    let mut buf = datagram.serialize().unwrap();
    // Type 0x21: STATUS + extensions
    assert_eq!(buf[0], 0x21);

    let deserialized = Datagram::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_status_with_default_priority() {
    let datagram = Datagram::new_status(1, 5, 10, None, None, ObjectStatus::DoesNotExist);

    let mut buf = datagram.serialize().unwrap();
    // Type 0x28: STATUS + DEFAULT_PRIORITY
    assert_eq!(buf[0], 0x28);

    let deserialized = Datagram::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, datagram);
    assert!(deserialized.publisher_priority.is_none());
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_status_plus_end_of_group_is_error() {
    // STATUS + END_OF_GROUP is a PROTOCOL_VIOLATION
    let result = ObjectDatagramType::from_properties(false, true, false, false, true);
    assert!(result.is_err());
  }

  #[test]
  fn test_invalid_type_is_error() {
    let result = ObjectDatagramType::try_from(0x10u64);
    assert!(result.is_err());
  }
}
