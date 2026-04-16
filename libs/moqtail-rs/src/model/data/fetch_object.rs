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

use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use crate::model::extension_header::object_extension::{
  ObjectExtension, deserialize_object_extensions,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::constant::ObjectStatus;

#[derive(Debug, Clone, PartialEq)]
pub struct FetchObject {
  pub group_id: u64,
  pub subgroup_id: u64,
  pub object_id: u64,
  pub publisher_priority: u8,
  pub extension_headers: Option<Vec<ObjectExtension>>,
  pub object_status: Option<ObjectStatus>,
  pub payload: Option<Bytes>,
}

impl FetchObject {
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();

    buf.put_vi(self.group_id)?;
    buf.put_vi(self.subgroup_id)?;
    buf.put_vi(self.object_id)?;
    buf.put_u8(self.publisher_priority);

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

  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let group_id = bytes.get_vi()?;
    let subgroup_id = bytes.get_vi()?;
    let object_id = bytes.get_vi()?;

    if bytes.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "FetchObject::deserialize",
        needed: 1,
        available: 0,
      });
    }
    let publisher_priority = bytes.get_u8();

    let ext_len = bytes.get_vi()?;
    let ext_len: usize =
      ext_len
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "ObjectDatagram::deserialize",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;
    let extension_headers = if ext_len > 0 {
      if bytes.remaining() < ext_len {
        return Err(ParseError::NotEnoughBytes {
          context: "ObjectDatagram::deserialize",
          needed: ext_len,
          available: bytes.remaining(),
        });
      }
      let mut header_bytes = bytes.copy_to_bytes(ext_len);
      let headers = deserialize_object_extensions(&mut header_bytes).map_err(|_| {
        ParseError::ProtocolViolation {
          context: "ObjectDatagram::deserialize(headers)",
          details: "Can't parse headers".to_string(),
        }
      })?;
      Some(headers)
    } else {
      None
    };

    let payload_len = bytes.get_vi()?;

    let (payload, object_status) = if payload_len == 0 {
      let status_raw = bytes.get_vi()?;
      let status = ObjectStatus::try_from(status_raw)?;
      (None, Some(status))
    } else {
      let payload_len: usize = payload_len
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "ObjectDatagram::deserialize",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;
      if bytes.remaining() < payload_len {
        return Err(ParseError::NotEnoughBytes {
          context: "ObjectDatagram::deserialize",
          needed: ext_len,
          available: bytes.remaining(),
        });
      }
      (Some(bytes.copy_to_bytes(payload_len)), None)
    };

    Ok(FetchObject {
      group_id,
      subgroup_id,
      object_id,
      publisher_priority,
      extension_headers,
      object_status,
      payload,
    })
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use crate::model::common::pair::KeyValuePair;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let group_id: u64 = 9;
    let subgroup_id = 144;
    let object_id: u64 = 10;
    let publisher_priority: u8 = 255;
    let extension_headers = Some(vec![
      ObjectExtension::Unknown {
        kvp: KeyValuePair::try_new_varint(0, 10).unwrap(),
      },
      ObjectExtension::Unknown {
        kvp: KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
      },
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

    let mut buf = fetch_object.serialize().unwrap();
    let deserialized = FetchObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, fetch_object);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let group_id: u64 = 9;
    let subgroup_id = 144;
    let object_id: u64 = 10;
    let publisher_priority: u8 = 255;
    let extension_headers = Some(vec![
      ObjectExtension::Unknown {
        kvp: KeyValuePair::try_new_varint(0, 10).unwrap(),
      },
      ObjectExtension::Unknown {
        kvp: KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
      },
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

    let serialized = fetch_object.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let deserialized = FetchObject::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, fetch_object);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let group_id: u64 = 9;
    let subgroup_id = 144;
    let object_id: u64 = 10;
    let publisher_priority: u8 = 255;
    let extension_headers = Some(vec![
      ObjectExtension::Unknown {
        kvp: KeyValuePair::try_new_varint(0, 10).unwrap(),
      },
      ObjectExtension::Unknown {
        kvp: KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
      },
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
    let buf = fetch_object.serialize().unwrap();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = FetchObject::deserialize(&mut partial);
    assert!(deserialized.is_err());
  }
}
