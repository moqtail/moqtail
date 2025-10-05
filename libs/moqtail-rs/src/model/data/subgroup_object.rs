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
use tracing::{debug, info};

use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;

use super::constant::ObjectStatus;

#[derive(Debug, Clone, PartialEq)]
pub struct SubgroupObject {
  pub object_id: u64,
  pub extension_headers: Option<Vec<KeyValuePair>>,
  pub object_status: Option<ObjectStatus>,
  pub payload: Option<Bytes>,
}

impl SubgroupObject {
  pub fn serialize(
    &self,
    previous_object_id: Option<u64>,
    _has_extensions: bool,
  ) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();

    let object_id_delta = if let Some(id) = previous_object_id {
      self.object_id - id - 1
    } else {
      self.object_id
    };

    debug!(
      "SubgroupObject::serialize || object_id_delta: {} prev: {:?} object_id: {}",
      object_id_delta, previous_object_id, self.object_id
    );

    buf.put_vi(object_id_delta)?;

    info!(
      "SubgroupObject::serialize || ext_headers: {:?}",
      &self.extension_headers
    );

    if let Some(ext_headers) = &self.extension_headers {
      if ext_headers.is_empty() {
        buf.put_vi(0)?;
      } else {
        let mut ext_buf = BytesMut::new();
        for header in ext_headers {
          ext_buf.extend_from_slice(&header.serialize()?);
        }
        buf.put_vi(ext_buf.len())?;
        buf.extend_from_slice(&ext_buf);
      }
    }

    if let Some(status) = self.object_status {
      buf.put_vi(0u64)?;
      buf.put_vi(status)?;
    } else if let Some(payload) = &self.payload {
      buf.put_vi(payload.len())?;
      buf.extend_from_slice(payload);
    } else {
      return Err(ParseError::ProtocolViolation {
        context: "SubgroupObject::serialize",
        details: "No object status, no payload".to_string(),
      });
    }

    Ok(buf.freeze())
  }

  pub fn deserialize(
    bytes: &mut Bytes,
    previous_object_id: Option<u64>,
    has_extensions: bool,
  ) -> Result<Self, ParseError> {
    let object_id_delta = bytes.get_vi()?;

    let object_id = if let Some(id) = previous_object_id {
      id + object_id_delta + 1
    } else {
      object_id_delta
    };

    debug!(
      "SubgroupObject::deserialize || object_id_delta: {} prev: {:?} object_id: {}",
      object_id_delta, previous_object_id, object_id
    );

    let extension_headers = if has_extensions {
      let ext_len = bytes.get_vi()?;

      if ext_len > 0 {
        let ext_len: usize =
          ext_len
            .try_into()
            .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
              context: "SubgroupObject::deserialize",
              from_type: "u64",
              to_type: "usize",
              details: e.to_string(),
            })?;

        if bytes.remaining() < ext_len {
          return Err(ParseError::NotEnoughBytes {
            context: "SubgroupObject::deserialize",
            needed: ext_len,
            available: bytes.remaining(),
          });
        }

        let mut header_bytes = bytes.copy_to_bytes(ext_len);
        let mut headers: Vec<KeyValuePair> = Vec::new();

        while header_bytes.has_remaining() {
          match KeyValuePair::deserialize(&mut header_bytes) {
            Ok(header) => headers.push(header),
            Err(_) => {
              return Err(ParseError::ProtocolViolation {
                context: "SubgroupObject::deserialize",
                details: "Failed to parse extension header".to_string(),
              });
            }
          }
        }

        Some(headers)
      } else {
        // TODO: this is a hack to deal with the fact that we can understand
        // whether we need to serialize this object with extensions
        Some(vec![])
      }
    } else {
      None
    };

    let payload_len = bytes.get_vi()?;

    let (object_status, payload) = if payload_len == 0 {
      let status_raw = bytes.get_vi()?;
      let status = ObjectStatus::try_from(status_raw)?;
      (Some(status), None)
    } else {
      let payload_len: usize = payload_len
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "SubgroupObject::deserialize",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;

      if bytes.remaining() < payload_len {
        return Err(ParseError::NotEnoughBytes {
          context: "SubgroupObject::deserialize",
          needed: payload_len,
          available: bytes.remaining(),
        });
      }
      let payload_data = bytes.copy_to_bytes(payload_len);
      (None, Some(payload_data))
    };

    Ok(SubgroupObject {
      object_id,
      extension_headers,
      object_status,
      payload,
    })
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let object_id: u64 = 10;
    let prev_object_id = 9;
    let extension_headers = Some(vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ]);
    let object_status = None;
    let payload = Some(Bytes::from_static(b"01239gjawkk92837aldmi"));

    let subgroup_object = SubgroupObject {
      object_id,
      extension_headers,
      payload,
      object_status,
    };

    let mut buf = subgroup_object
      .serialize(Some(prev_object_id), true)
      .unwrap();
    let deserialized = SubgroupObject::deserialize(&mut buf, Some(prev_object_id), true).unwrap();
    assert_eq!(deserialized, subgroup_object);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let object_id: u64 = 10;
    let prev_object_id = 9;
    let extension_headers = Some(vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ]);
    let object_status = None;
    let payload = Some(Bytes::from_static(b"01239gjawkk92837aldmi"));

    let subgroup_object = SubgroupObject {
      object_id,
      extension_headers,
      payload,
      object_status,
    };

    let serialized = subgroup_object
      .serialize(Some(prev_object_id), true)
      .unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let deserialized = SubgroupObject::deserialize(&mut buf, Some(prev_object_id), true).unwrap();
    assert_eq!(deserialized, subgroup_object);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let object_id: u64 = 10;
    let prev_object_id = 9;
    let extension_headers = Some(vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ]);
    let object_status = None;
    let payload = Some(Bytes::from_static(b"01239gjawkk92837aldmi"));

    let subgroup_object = SubgroupObject {
      object_id,
      extension_headers,
      payload,
      object_status,
    };

    let buf = subgroup_object
      .serialize(Some(prev_object_id), true)
      .unwrap();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = SubgroupObject::deserialize(&mut partial, Some(prev_object_id), true);
    assert!(deserialized.is_err());
  }
}
