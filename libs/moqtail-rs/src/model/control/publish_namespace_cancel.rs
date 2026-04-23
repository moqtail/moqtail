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

use bytes::{BufMut, Bytes, BytesMut};

use super::constant::RequestErrorCode;
use super::control_message::ControlMessageTrait;
use crate::model::common::reason_phrase::ReasonPhrase;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::control::constant::ControlMessageType;
use crate::model::error::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub struct PublishNamespaceCancel {
  pub request_id: u64,
  pub error_code: RequestErrorCode,
  pub reason_phrase: ReasonPhrase,
}
impl PublishNamespaceCancel {
  pub fn new(request_id: u64, error_code: RequestErrorCode, reason_phrase: ReasonPhrase) -> Self {
    PublishNamespaceCancel {
      request_id,
      error_code,
      reason_phrase,
    }
  }
}
impl ControlMessageTrait for PublishNamespaceCancel {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::PublishNamespaceCancel)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    let error_code_u64: u64 = self.error_code.into();
    payload.put_vi(error_code_u64)?;
    payload.extend_from_slice(&self.reason_phrase.serialize()?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "PublishNamespaceCancel::serialize(payload_length)",
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
    let error_code = RequestErrorCode::try_from(error_code_raw)?;
    let reason_phrase = ReasonPhrase::deserialize(payload)?;

    Ok(Box::new(PublishNamespaceCancel {
      request_id,
      error_code,
      reason_phrase,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::PublishNamespaceCancel
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let error_code = RequestErrorCode::ExpiredAuthToken;
    let reason_phrase = ReasonPhrase::try_new("why are you running?".to_string()).unwrap();
    let request_id = 42;
    let sub_update = PublishNamespaceCancel {
      request_id,
      error_code,
      reason_phrase,
    };

    let mut buf = sub_update.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishNamespaceCancel as u64);

    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let deserialized = PublishNamespaceCancel::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, sub_update);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let error_code = RequestErrorCode::InternalError;
    let reason_phrase = ReasonPhrase::try_new("bomboclad".to_string()).unwrap();
    let request_id = 1337;
    let sub_update = PublishNamespaceCancel {
      request_id,
      error_code,
      reason_phrase,
    };

    let serialized = sub_update.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);

    let mut buf = excess.freeze();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishNamespaceCancel as u64);

    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining() - 3);

    let deserialized = PublishNamespaceCancel::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, sub_update);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let error_code = RequestErrorCode::MalformedAuthToken;
    let reason_phrase = ReasonPhrase::try_new("Uvuvwevwevwe".to_string()).unwrap();
    let request_id = 99;
    let sub_update = PublishNamespaceCancel {
      request_id,
      error_code,
      reason_phrase,
    };

    let mut buf = sub_update.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::PublishNamespaceCancel as u64);

    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = PublishNamespaceCancel::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
