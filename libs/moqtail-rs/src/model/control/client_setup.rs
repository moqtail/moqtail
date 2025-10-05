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
use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{BufMut, Bytes, BytesMut};

#[derive(Debug, Clone, PartialEq)]
pub struct ClientSetup {
  pub supported_versions: Vec<u32>,
  pub setup_parameters: Vec<KeyValuePair>,
}

impl ClientSetup {
  //TODO: Assert supported versions and params are not empty during initialization
  pub fn new(supported_versions: Vec<u32>, setup_parameters: Vec<KeyValuePair>) -> Self {
    ClientSetup {
      supported_versions,
      setup_parameters,
    }
  }
}

impl ControlMessageTrait for ClientSetup {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::ClientSetup)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.supported_versions.len())?;
    for version in &self.supported_versions {
      payload.put_vi(*version)?;
    }
    payload.put_vi(self.setup_parameters.len())?;
    for param in &self.setup_parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "ClientSetup::serialize(payload_length)",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;

    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);

    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let number_of_supported_versions = payload.get_vi()?;

    if number_of_supported_versions == 0 {
      return Err(ParseError::ProtocolViolation {
        context: "ClientSetup::parse_payload(number_of_supported_versions)",
        details: "Must support at least one version".to_string(),
      });
    }

    let mut supported_versions = Vec::new();

    for _ in 0..number_of_supported_versions {
      let ver = payload.get_vi()?;

      let supported_version: u32 =
        ver
          .try_into()
          .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
            context: "ClientSetup::parse_payload(supported_version)",
            from_type: "u64",
            to_type: "u32",
            details: e.to_string(),
          })?;
      supported_versions.push(supported_version);
    }

    let number_of_parameters = payload.get_vi()?;
    let mut setup_parameters = Vec::new();

    for _ in 0..number_of_parameters {
      let param = KeyValuePair::deserialize(payload)?;
      setup_parameters.push(param);
    }

    Ok(Box::new(ClientSetup {
      supported_versions,
      setup_parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::ClientSetup
  }
}

#[cfg(test)]
mod tests {
  use crate::model::control::constant::DRAFT_14;

  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let supported_versions = vec![12345, DRAFT_14];
    let setup_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"Set me up!")).unwrap(),
    ];
    let client_setup = ClientSetup {
      supported_versions,
      setup_parameters,
    };

    let mut buf = client_setup.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::ClientSetup as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = ClientSetup::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, client_setup);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let supported_versions = vec![12345, DRAFT_14];
    let setup_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"Set me up!")).unwrap(),
    ];
    let client_setup = ClientSetup {
      supported_versions,
      setup_parameters,
    };

    let serialized = client_setup.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::ClientSetup as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = ClientSetup::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, client_setup);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let supported_versions = vec![12345, DRAFT_14];
    let setup_parameters: Vec<KeyValuePair> = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"Set me up!")).unwrap(),
    ];
    let client_setup = ClientSetup {
      supported_versions,
      setup_parameters,
    };

    let mut buf = client_setup.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::ClientSetup as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = ClientSetup::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
