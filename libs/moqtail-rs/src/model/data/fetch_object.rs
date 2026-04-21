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

use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::constant::{FetchObjectPriorState, FetchObjectSerializationFlags, ObjectStatus};

/// End-of-range marker on a fetch stream.
#[derive(Debug, Clone, PartialEq)]
pub enum FetchStreamItem {
  Object(FetchObject),
  EndOfNonExistentRange { group_id: u64, object_id: u64 },
  EndOfUnknownRange { group_id: u64, object_id: u64 },
}

impl FetchStreamItem {
  /// Serialize this item. For end-of-range markers, only flags + Group ID + Object ID
  /// are written (spec §10.4.4.2: Subgroup ID, Priority and Extensions are not present).
  pub fn serialize(&self, prior: Option<&FetchObjectPriorState>) -> Result<Bytes, ParseError> {
    match self {
      FetchStreamItem::Object(obj) => obj.serialize(prior),
      FetchStreamItem::EndOfNonExistentRange {
        group_id,
        object_id,
      } => {
        let mut buf = BytesMut::new();
        buf.put_vi(FetchObjectSerializationFlags::END_OF_NON_EXISTENT_RANGE)?;
        buf.put_vi(*group_id)?;
        buf.put_vi(*object_id)?;
        Ok(buf.freeze())
      }
      FetchStreamItem::EndOfUnknownRange {
        group_id,
        object_id,
      } => {
        let mut buf = BytesMut::new();
        buf.put_vi(FetchObjectSerializationFlags::END_OF_UNKNOWN_RANGE)?;
        buf.put_vi(*group_id)?;
        buf.put_vi(*object_id)?;
        Ok(buf.freeze())
      }
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchObject {
  pub group_id: u64,
  pub subgroup_id: Option<u64>,
  pub object_id: u64,
  pub publisher_priority: u8,
  pub extension_headers: Option<Vec<KeyValuePair>>,
  pub object_status: Option<ObjectStatus>,
  pub payload: Option<Bytes>,
}

impl FetchObject {
  /// Serialize this object with optional delta encoding based on prior object state.
  pub fn serialize(&self, prior: Option<&FetchObjectPriorState>) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();

    // Compute serialization flags
    let has_extensions = self
      .extension_headers
      .as_ref()
      .is_some_and(|v| !v.is_empty());
    let flags = FetchObjectSerializationFlags::from_properties(
      prior,
      self.group_id,
      self.subgroup_id,
      self.object_id,
      self.publisher_priority,
      has_extensions,
    );

    // Write serialization flags
    buf.put_vi(flags.0)?;

    // Write Group ID if explicit
    if flags.has_explicit_group_id() {
      buf.put_vi(self.group_id)?;
    }

    // Write Subgroup ID if not datagram
    if !flags.is_datagram() {
      let mode = flags.subgroup_mode();
      if mode == 0x03 {
        // Explicit subgroup ID
        buf.put_vi(self.subgroup_id.unwrap_or(0))?;
      }
      // modes 0x00, 0x01, 0x02 don't write the field
    }

    // Write Object ID if explicit
    if flags.has_explicit_object_id() {
      buf.put_vi(self.object_id)?;
    }

    // Write Publisher Priority if explicit
    if flags.has_explicit_priority() {
      buf.put_u8(self.publisher_priority);
    }

    // Write Extensions if present
    if flags.has_extensions() {
      if let Some(ext_headers) = &self.extension_headers {
        let mut ext_buf = BytesMut::new();
        for header in ext_headers {
          ext_buf.extend_from_slice(&header.serialize()?);
        }
        buf.put_vi(ext_buf.len() as u64)?;
        buf.extend_from_slice(&ext_buf);
      } else {
        buf.put_vi(0u64)?;
      }
    }

    // Write payload or status
    if let Some(status) = self.object_status {
      buf.put_vi(0u64)?;
      buf.put_vi(status)?;
    } else if let Some(payload) = &self.payload {
      buf.put_vi(payload.len() as u64)?;
      buf.extend_from_slice(payload);
    } else {
      return Err(ParseError::ProtocolViolation {
        context: "FetchObject::serialize",
        details: "No object status, no payload".to_string(),
      });
    }

    Ok(buf.freeze())
  }

  /// Deserialize a fetch stream item (object or end-of-range marker).
  /// Updates the prior state with the deserialized values.
  pub fn deserialize_item(
    bytes: &mut Bytes,
    prior: &mut Option<FetchObjectPriorState>,
  ) -> Result<FetchStreamItem, ParseError> {
    let flags_raw = bytes.get_vi()?;
    let flags = FetchObjectSerializationFlags::try_from(flags_raw)?;

    // Handle end-of-range markers
    if flags.is_end_of_range() {
      let group_id = bytes.get_vi()?;
      let object_id = bytes.get_vi()?;
      let item = if flags.0 == FetchObjectSerializationFlags::END_OF_NON_EXISTENT_RANGE {
        FetchStreamItem::EndOfNonExistentRange {
          group_id,
          object_id,
        }
      } else {
        FetchStreamItem::EndOfUnknownRange {
          group_id,
          object_id,
        }
      };
      return Ok(item);
    }

    // Validate first-object constraints
    if prior.is_none() {
      if !flags.has_explicit_group_id() {
        return Err(ParseError::ProtocolViolation {
          context: "FetchObject::deserialize_item",
          details: "First object must have explicit Group ID (bit 0x08 set)".to_string(),
        });
      }
      if !flags.is_datagram() {
        let mode = flags.subgroup_mode();
        if mode == 0x01 || mode == 0x02 {
          return Err(ParseError::ProtocolViolation {
            context: "FetchObject::deserialize_item",
            details: format!("First object cannot use subgroup mode {mode} (references prior)"),
          });
        }
      }
      if !flags.has_explicit_object_id() {
        return Err(ParseError::ProtocolViolation {
          context: "FetchObject::deserialize_item",
          details: "First object must have explicit Object ID (bit 0x04 set)".to_string(),
        });
      }
      if !flags.has_explicit_priority() {
        return Err(ParseError::ProtocolViolation {
          context: "FetchObject::deserialize_item",
          details: "First object must have explicit Priority (bit 0x10 set)".to_string(),
        });
      }
    }

    // Read Group ID
    let group_id = if flags.has_explicit_group_id() {
      bytes.get_vi()?
    } else {
      prior.as_ref().unwrap().group_id
    };

    // Read Subgroup ID
    let subgroup_id = if flags.is_datagram() {
      None
    } else {
      let mode = flags.subgroup_mode();

      match mode {
        0x00 => Some(0),
        0x01 => prior.as_ref().unwrap().subgroup_id,
        0x02 => prior.as_ref().unwrap().subgroup_id.map(|s| s + 1),
        0x03 => Some(bytes.get_vi()?),
        _ => unreachable!(), // only 4 modes
      }
    };

    // Read Object ID
    let object_id = if flags.has_explicit_object_id() {
      bytes.get_vi()?
    } else {
      prior.as_ref().unwrap().object_id + 1
    };

    // Read Publisher Priority
    let publisher_priority = if flags.has_explicit_priority() {
      bytes.get_u8()
    } else {
      prior.as_ref().unwrap().publisher_priority
    };

    // Read Extensions
    let extension_headers = if flags.has_extensions() {
      let ext_len = bytes.get_vi()?;
      let ext_len: usize =
        ext_len
          .try_into()
          .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
            context: "FetchObject::deserialize_item",
            from_type: "u64",
            to_type: "usize",
            details: e.to_string(),
          })?;
      if ext_len > 0 {
        if bytes.remaining() < ext_len {
          return Err(ParseError::NotEnoughBytes {
            context: "FetchObject::deserialize_item",
            needed: ext_len,
            available: bytes.remaining(),
          });
        }
        let mut header_bytes = bytes.copy_to_bytes(ext_len);
        let mut headers: Vec<KeyValuePair> = Vec::new();
        while header_bytes.has_remaining() {
          let h = KeyValuePair::deserialize(&mut header_bytes).map_err(|_| {
            ParseError::ProtocolViolation {
              context: "FetchObject::deserialize_item(headers)",
              details: "Can't parse headers".to_string(),
            }
          })?;
          headers.push(h);
        }
        Some(headers)
      } else {
        None
      }
    } else {
      None
    };

    // Read payload or status
    let payload_len = bytes.get_vi()?;
    let (payload, object_status) = if payload_len == 0 {
      let status_raw = bytes.get_vi()?;
      let status = ObjectStatus::try_from(status_raw)?;
      (None, Some(status))
    } else {
      let payload_len: usize = payload_len
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "FetchObject::deserialize_item",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;
      if bytes.remaining() < payload_len {
        return Err(ParseError::NotEnoughBytes {
          context: "FetchObject::deserialize_item",
          needed: payload_len,
          available: bytes.remaining(),
        });
      }
      (Some(bytes.copy_to_bytes(payload_len)), None)
    };

    let obj = FetchObject {
      group_id,
      subgroup_id,
      object_id,
      publisher_priority,
      extension_headers,
      object_status,
      payload,
    };

    // Update prior state
    *prior = Some(obj.to_prior_state());

    Ok(FetchStreamItem::Object(obj))
  }

  /// Deserialize a single fetch object (convenience method for tests).
  /// Note: This cannot be used for multiple objects on the same stream, as it doesn't track prior state.
  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let mut prior = None;
    match FetchObject::deserialize_item(bytes, &mut prior)? {
      FetchStreamItem::Object(obj) => Ok(obj),
      _ => Err(ParseError::ProtocolViolation {
        context: "FetchObject::deserialize",
        details: "Cannot deserialize end-of-range marker as object".to_string(),
      }),
    }
  }

  /// Convert this object to prior state for use in delta encoding the next object.
  pub fn to_prior_state(&self) -> FetchObjectPriorState {
    FetchObjectPriorState {
      group_id: self.group_id,
      subgroup_id: self.subgroup_id,
      object_id: self.object_id,
      publisher_priority: self.publisher_priority,
    }
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use crate::model::common::varint::BufMutVarIntExt;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let group_id: u64 = 9;
    let subgroup_id = Some(144);
    let object_id: u64 = 10;
    let publisher_priority: u8 = 255;
    let extension_headers = Some(vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ]);
    let object_status = None;
    let payload = Some(Bytes::from_static(b"01239gjawkk92837aldmi"));

    let fetch_object = FetchObject {
      group_id,
      subgroup_id,
      object_id,
      publisher_priority,
      extension_headers,
      payload,
      object_status,
    };

    let mut buf = fetch_object.serialize(None).unwrap();
    let deserialized = FetchObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, fetch_object);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let group_id: u64 = 9;
    let subgroup_id = Some(144);
    let object_id: u64 = 10;
    let publisher_priority: u8 = 255;
    let extension_headers = Some(vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ]);
    let object_status = None;
    let payload = Some(Bytes::from_static(b"01239gjawkk92837aldmi"));

    let fetch_object = FetchObject {
      group_id,
      subgroup_id,
      object_id,
      publisher_priority,
      extension_headers,
      payload,
      object_status,
    };

    let serialized = fetch_object.serialize(None).unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let deserialized = FetchObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, fetch_object);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_datagram_roundtrip() {
    let obj = FetchObject {
      group_id: 5,
      subgroup_id: None, // datagram-forwarded object
      object_id: 3,
      publisher_priority: 128,
      extension_headers: None,
      object_status: None,
      payload: Some(Bytes::from_static(b"datagram")),
    };
    let mut buf = obj.serialize(None).unwrap();
    // Flags must have bit 0x40 set (datagram) and no subgroup ID on wire
    let flags = FetchObjectSerializationFlags(buf.clone().get_vi().unwrap());
    assert!(flags.is_datagram());
    let deserialized = FetchObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, obj);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_end_of_non_existent_range_roundtrip() {
    let item = FetchStreamItem::EndOfNonExistentRange {
      group_id: 42,
      object_id: 7,
    };
    let mut buf = item.serialize(None).unwrap();
    let mut prior = None;
    let deserialized = FetchObject::deserialize_item(&mut buf, &mut prior).unwrap();
    assert_eq!(deserialized, item);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_end_of_unknown_range_roundtrip() {
    let item = FetchStreamItem::EndOfUnknownRange {
      group_id: 99,
      object_id: 0,
    };
    let mut buf = item.serialize(None).unwrap();
    let mut prior = None;
    let deserialized = FetchObject::deserialize_item(&mut buf, &mut prior).unwrap();
    assert_eq!(deserialized, item);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_invalid_flags_rejected() {
    // 0x80 is invalid (>= 128, not 0x8C or 0x10C)
    let mut buf = BytesMut::new();
    buf.put_vi(0x80u64).unwrap();
    let mut bytes = buf.freeze();
    let mut prior = None;
    let result = FetchObject::deserialize_item(&mut bytes, &mut prior);
    assert!(result.is_err());

    // 0x10D is also invalid
    let mut buf = BytesMut::new();
    buf.put_vi(0x10Du64).unwrap();
    let mut bytes = buf.freeze();
    let mut prior = None;
    let result = FetchObject::deserialize_item(&mut bytes, &mut prior);
    assert!(result.is_err());
  }

  #[test]
  fn test_first_object_must_have_explicit_fields() {
    // Build a valid first object, then tamper with flags to remove explicit Group ID
    let obj = FetchObject {
      group_id: 1,
      subgroup_id: Some(0),
      object_id: 0,
      publisher_priority: 10,
      extension_headers: None,
      object_status: None,
      payload: Some(Bytes::from_static(b"hello")),
    };
    let serialized = obj.serialize(None).unwrap();

    // Deserializing with no prior should succeed (all explicit flags set for first object)
    let mut buf = serialized.clone();
    let mut prior = None;
    assert!(FetchObject::deserialize_item(&mut buf, &mut prior).is_ok());

    // Manually craft flags with no explicit Group ID (bit 0x08 unset) for first object
    let mut tampered = BytesMut::new();
    tampered.put_vi(0x17u64).unwrap(); // has explicit obj id (0x04) + priority (0x10) + extensions (0x20) but NOT group id
    let mut bytes = tampered.freeze();
    let mut prior = None;
    let result = FetchObject::deserialize_item(&mut bytes, &mut prior);
    assert!(matches!(result, Err(ParseError::ProtocolViolation { .. })));
  }

  #[test]
  fn test_first_object_bad_subgroup_mode_rejected() {
    // Mode 0x01 ("same as prior") on first object must be rejected
    let mut buf = BytesMut::new();
    // flags: explicit group (0x08) + explicit obj (0x04) + explicit priority (0x10) + subgroup mode=0x01
    buf.put_vi(0x1Du64).unwrap(); // 0x08|0x04|0x10|0x01 = 0x1D
    let mut bytes = buf.freeze();
    let mut prior = None;
    let result = FetchObject::deserialize_item(&mut bytes, &mut prior);
    assert!(matches!(result, Err(ParseError::ProtocolViolation { .. })));
  }

  #[test]
  fn test_partial_message() {
    let group_id: u64 = 9;
    let subgroup_id = Some(144);
    let object_id: u64 = 10;
    let publisher_priority: u8 = 255;
    let extension_headers = Some(vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ]);
    let object_status = None;
    let payload = Some(Bytes::from_static(b"01239gjawkk92837aldmi"));

    let fetch_object = FetchObject {
      group_id,
      subgroup_id,
      object_id,
      publisher_priority,
      extension_headers,
      payload,
      object_status,
    };
    let buf = fetch_object.serialize(None).unwrap();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = FetchObject::deserialize(&mut partial);
    assert!(deserialized.is_err());
  }
}
