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

use super::constant::{ControlMessageType, SubscribeNamespaceErrorCode};
use super::control_message::ControlMessageTrait;
use crate::model::common::reason_phrase::ReasonPhrase;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct SubscribeNamespaceError {
  pub request_id: u64,
  pub error_code: SubscribeNamespaceErrorCode,
  pub reason_phrase: ReasonPhrase,
}

impl SubscribeNamespaceError {
  pub fn new(
    request_id: u64,
    error_code: SubscribeNamespaceErrorCode,
    reason_phrase: ReasonPhrase,
  ) -> Self {
    Self {
      request_id,
      error_code,
      reason_phrase,
    }
  }
}

impl ControlMessageTrait for SubscribeNamespaceError {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::SubscribeNamespaceError)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.put_vi(self.error_code)?;
    payload.extend_from_slice(&self.reason_phrase.serialize()?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "SubscribeNamespaceError::serialize(payload_length)",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;

    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);

    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let request_id = payload.get_vi()?;

    let error_code_raw = payload.get_vi()?;
    let error_code = SubscribeNamespaceErrorCode::try_from(error_code_raw)?;

    let reason_phrase = ReasonPhrase::deserialize(payload)?;

    Ok(Box::new(SubscribeNamespaceError {
      request_id,
      error_code,
      reason_phrase,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::SubscribeNamespaceError
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let request_id = 662607;
    let error_code = SubscribeNamespaceErrorCode::ExpiredAuthToken;
    let reason_phrase = ReasonPhrase::try_new("Cheap weiners on aisle 9".to_string()).unwrap();
    let subscribe_namespace_error = SubscribeNamespaceError {
      request_id,
      error_code,
      reason_phrase,
    };
    let mut buf = subscribe_namespace_error.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeNamespaceError as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = SubscribeNamespaceError::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe_namespace_error);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let request_id = 662607;
    let error_code = SubscribeNamespaceErrorCode::ExpiredAuthToken;
    let reason_phrase = ReasonPhrase::try_new("Cheap weiners on aisle 9".to_string()).unwrap();
    let subscribe_namespace_error = SubscribeNamespaceError {
      request_id,
      error_code,
      reason_phrase,
    };

    let serialized = subscribe_namespace_error.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeNamespaceError as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = SubscribeNamespaceError::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, subscribe_namespace_error);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let request_id = 662607;
    let error_code = SubscribeNamespaceErrorCode::ExpiredAuthToken;
    let reason_phrase = ReasonPhrase::try_new("Cheap weiners on aisle 9".to_string()).unwrap();
    let subscribe_namespace_error = SubscribeNamespaceError {
      request_id,
      error_code,
      reason_phrase,
    };
    let mut buf = subscribe_namespace_error.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::SubscribeNamespaceError as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = SubscribeNamespaceError::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
