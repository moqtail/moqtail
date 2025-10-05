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

use super::constant::{ControlMessageType, PublishErrorCode};
use super::control_message::ControlMessageTrait;
use crate::model::common::reason_phrase::ReasonPhrase;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct PublishError {
  pub request_id: u64,
  pub error_code: PublishErrorCode,
  pub reason_phrase: ReasonPhrase,
}

impl PublishError {
  pub fn new(request_id: u64, error_code: PublishErrorCode, reason_phrase: ReasonPhrase) -> Self {
    Self {
      request_id,
      error_code,
      reason_phrase,
    }
  }
}

impl ControlMessageTrait for PublishError {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::PublishError)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.put_vi(self.error_code as u64)?;
    payload.extend_from_slice(&self.reason_phrase.serialize()?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "PublishError::serialize(payload_length)",
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
    let error_code = PublishErrorCode::try_from(error_code_raw)?;
    let reason_phrase = ReasonPhrase::deserialize(payload)?;

    Ok(Box::new(PublishError {
      request_id,
      error_code,
      reason_phrase,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::PublishError
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let request_id = 12345;
    let error_code = PublishErrorCode::Unauthorized;
    let reason_phrase = ReasonPhrase::try_new("Not authorized to publish".to_string()).unwrap();
    let publish_error = PublishError::new(request_id, error_code, reason_phrase);

    let mut buf = publish_error.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishError as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = PublishError::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, publish_error);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_all_error_codes() {
    let error_codes = vec![
      PublishErrorCode::InternalError,
      PublishErrorCode::Unauthorized,
      PublishErrorCode::Timeout,
      PublishErrorCode::NotSupported,
      PublishErrorCode::Uninterested,
    ];

    for error_code in error_codes {
      let request_id = 123;
      let reason_phrase = ReasonPhrase::try_new("Test error".to_string()).unwrap();
      let publish_error = PublishError::new(request_id, error_code, reason_phrase);

      let mut buf = publish_error.serialize().unwrap();
      let msg_type = buf.get_vi().unwrap();
      assert_eq!(msg_type, ControlMessageType::PublishError as u64);
      let msg_length = buf.get_u16();
      assert_eq!(msg_length as usize, buf.remaining());
      let deserialized = PublishError::parse_payload(&mut buf).unwrap();
      assert_eq!(*deserialized, publish_error);
    }
  }

  #[test]
  fn test_partial_message() {
    let request_id = 12345;
    let error_code = PublishErrorCode::Timeout;
    let reason_phrase = ReasonPhrase::try_new("Request timed out".to_string()).unwrap();
    let publish_error = PublishError::new(request_id, error_code, reason_phrase);

    let mut buf = publish_error.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishError as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let result = PublishError::parse_payload(&mut partial);
    assert!(result.is_err());
  }
}
