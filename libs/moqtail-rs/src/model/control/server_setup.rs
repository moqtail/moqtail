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

use super::control_message::ControlMessageTrait;
use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{BufMut, Bytes, BytesMut};

use super::constant::ControlMessageType;

#[derive(Debug, PartialEq, Clone)]
pub struct ServerSetup {
  pub selected_version: u32,
  pub setup_parameters: Vec<KeyValuePair>,
}

impl ServerSetup {
  pub fn new(selected_version: u32, setup_parameters: Vec<KeyValuePair>) -> Self {
    Self {
      selected_version,
      setup_parameters,
    }
  }
}

impl ControlMessageTrait for ServerSetup {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::ServerSetup)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.selected_version)?;
    payload.put_vi(self.setup_parameters.len())?;

    for param in &self.setup_parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "ServerSetup::serialize(payload_length)",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;

    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);
    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let selected_version = payload.get_vi()? as u32;

    let number_of_parameters = payload.get_vi()?;
    let mut setup_parameters = Vec::new();

    for _ in 0..number_of_parameters {
      let param = KeyValuePair::deserialize(payload)?;
      setup_parameters.push(param);
    }

    Ok(Box::new(ServerSetup {
      selected_version,
      setup_parameters,
    }))
  }
  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::ServerSetup
  }
}

#[cfg(test)]
mod tests {
  use crate::model::control::constant::DRAFT_14;

  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let selected_version = DRAFT_14;
    let setup_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"Set me up!")).unwrap(),
    ];
    let server_setup = ServerSetup {
      selected_version,
      setup_parameters,
    };

    let mut buf = server_setup.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::ServerSetup as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = ServerSetup::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, server_setup);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let selected_version = DRAFT_14;
    let setup_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"Set me up!")).unwrap(),
    ];
    let server_setup = ServerSetup {
      selected_version,
      setup_parameters,
    };

    let serialized = server_setup.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::ServerSetup as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = ServerSetup::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, server_setup);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let selected_version = DRAFT_14;
    let setup_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"Set me up!")).unwrap(),
    ];
    let server_setup = ServerSetup {
      selected_version,
      setup_parameters,
    };

    let mut buf = server_setup.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::ServerSetup as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = ServerSetup::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
