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

use super::constant::ControlMessageType;
use super::control_message::ControlMessageTrait;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

/// The New Session URI is bounded at 8,192 bytes; a longer one is a protocol
/// violation.
const MAX_NEW_SESSION_URI_LEN: usize = 8192;

#[derive(Debug, PartialEq, Clone)]
pub struct GoAway {
  /// Empty means the current URI is reused.
  pub new_session_uri: Option<String>,
  /// Milliseconds the sender waits for graceful closure (0 = no specific timeout).
  pub timeout: u64,
  /// The smallest peer Request ID not necessarily processed. Present only when
  /// the GOAWAY is sent on the control stream.
  pub request_id: Option<u64>,
}

impl GoAway {
  pub fn new(new_session_uri: Option<String>, timeout: u64, request_id: Option<u64>) -> Self {
    let new_session_uri = new_session_uri.filter(|uri| !uri.is_empty());
    Self {
      new_session_uri,
      timeout,
      request_id,
    }
  }
}

impl ControlMessageTrait for GoAway {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::GoAway)?;

    let mut payload = BytesMut::new();
    match &self.new_session_uri {
      Some(uri) => {
        payload.put_vi(uri.len())?;
        payload.extend_from_slice(uri.as_bytes());
      }
      None => {
        payload.put_vi(0)?;
      }
    }
    payload.put_vi(self.timeout)?;
    if let Some(request_id) = self.request_id {
      payload.put_vi(request_id)?;
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "GoAway::serialize(payload_length)",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;
    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);

    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let uri_length: usize =
      payload
        .get_vi()?
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "GoAway::parse_payload(uri_length)",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;

    if uri_length > MAX_NEW_SESSION_URI_LEN {
      return Err(ParseError::ProtocolViolation {
        context: "GoAway::parse_payload(uri_length)",
        details: format!("New Session URI length {uri_length} exceeds {MAX_NEW_SESSION_URI_LEN}"),
      });
    }

    let new_session_uri = if uri_length == 0 {
      None
    } else {
      if payload.remaining() < uri_length {
        return Err(ParseError::NotEnoughBytes {
          context: "GoAway::parse_payload(uri_length)",
          needed: uri_length,
          available: payload.remaining(),
        });
      }
      let bytes = payload.copy_to_bytes(uri_length);
      Some(
        String::from_utf8(bytes.to_vec()).map_err(|e| ParseError::InvalidUTF8 {
          context: "GoAway::parse_payload(new_session_uri)",
          details: e.to_string(),
        })?,
      )
    };

    let timeout = payload.get_vi()?;

    // Request ID is present only when the GOAWAY is sent on the control stream,
    // so it is optional and trailing (bounded by the outer Length field).
    let request_id = if payload.has_remaining() {
      Some(payload.get_vi()?)
    } else {
      None
    };

    Ok(Box::new(GoAway {
      new_session_uri,
      timeout,
      request_id,
    }))
  }
  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::GoAway
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::Buf;

  fn roundtrip(go_away: GoAway) {
    let mut buf = go_away.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::GoAway as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = GoAway::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, go_away);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn roundtrip_without_request_id() {
    roundtrip(GoAway::new(
      Some("moqt://new.example".to_string()),
      5000,
      None,
    ));
  }

  #[test]
  fn roundtrip_with_request_id() {
    roundtrip(GoAway::new(
      Some("moqt://new.example".to_string()),
      5000,
      Some(42),
    ));
  }

  #[test]
  fn roundtrip_empty_uri_reuses_current() {
    let go_away = GoAway::new(Some(String::new()), 0, Some(7));
    assert_eq!(go_away.new_session_uri, None);
    roundtrip(go_away);
  }

  #[test]
  fn test_excess_roundtrip() {
    let go_away = GoAway::new(Some("moqt://new.example".to_string()), 5000, Some(42));

    let serialized = go_away.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::GoAway as u64);
    let msg_length = buf.get_u16();
    // The trailing request_id is bounded by Length, so slice the payload first.
    let mut payload = buf.copy_to_bytes(msg_length as usize);
    let deserialized = GoAway::parse_payload(&mut payload).unwrap();
    assert_eq!(*deserialized, go_away);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let go_away = GoAway::new(Some("moqt://new.example".to_string()), 5000, Some(42));
    let mut buf = go_away.serialize().unwrap();
    let _ = buf.get_vi().unwrap();
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    assert!(GoAway::parse_payload(&mut partial).is_err());
  }
}
