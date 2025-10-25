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

use crate::model::{
  common::{pair::KeyValuePair, varint::BufMutVarIntExt},
  error::ParseError,
  parameter::constant::{TokenAliasType, VersionSpecificParameterType},
};

use bytes::{Bytes, BytesMut};

use crate::model::common::varint::BufVarIntExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionParameter {
  MaxCacheDuration {
    duration: u64,
  },
  DeliveryTimeout {
    object_timeout: u64,
  },
  AuthorizationToken {
    alias_type: u64,
    token_alias: Option<u64>,
    token_type: Option<u64>,
    token_value: Option<Bytes>,
  },
  ForwardActionGroup {
    group_id: u64,
  },
}

impl VersionParameter {
  pub fn new_max_cache_duration(duration: u64) -> Self {
    VersionParameter::MaxCacheDuration { duration }
  }

  pub fn new_delivery_timeout(object_timeout: u64) -> Self {
    VersionParameter::DeliveryTimeout { object_timeout }
  }

  pub fn new_auth_token_delete(token_alias: u64) -> Self {
    VersionParameter::AuthorizationToken {
      alias_type: TokenAliasType::Delete as u64,
      token_alias: Some(token_alias),
      token_type: None,
      token_value: None,
    }
  }

  pub fn new_forward_action_group(group_id: u64) -> Self {
    VersionParameter::ForwardActionGroup { group_id }
  }

  pub fn new_auth_token_register(token_alias: u64, token_type: u64, token_value: Bytes) -> Self {
    VersionParameter::AuthorizationToken {
      alias_type: TokenAliasType::Register as u64,
      token_alias: Some(token_alias),
      token_type: Some(token_type),
      token_value: Some(token_value),
    }
  }

  pub fn new_auth_token_use_alias(token_alias: u64) -> Self {
    VersionParameter::AuthorizationToken {
      alias_type: TokenAliasType::UseAlias as u64,
      token_alias: Some(token_alias),
      token_type: None,
      token_value: None,
    }
  }

  pub fn new_auth_token_use_value(token_type: u64, token_value: Bytes) -> Self {
    VersionParameter::AuthorizationToken {
      alias_type: TokenAliasType::UseValue as u64,
      token_alias: None,
      token_type: Some(token_type),
      token_value: Some(token_value),
    }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut bytes = BytesMut::new();
    match self {
      VersionParameter::MaxCacheDuration { duration } => {
        let kvp = KeyValuePair::try_new_varint(
          VersionSpecificParameterType::MaxCacheDuration as u64,
          *duration,
        )?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
      VersionParameter::DeliveryTimeout { object_timeout } => {
        let kvp = KeyValuePair::try_new_varint(
          VersionSpecificParameterType::DeliveryTimeout as u64,
          *object_timeout,
        )?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
      VersionParameter::AuthorizationToken {
        alias_type,
        token_alias,
        token_type,
        token_value,
      } => {
        let mut payload_bytes = BytesMut::new();
        payload_bytes.put_vi(*alias_type)?;
        match *alias_type {
          x if x == TokenAliasType::Delete as u64 => {
            if let Some(token_alias) = token_alias {
              payload_bytes.put_vi(*token_alias)?;
            }
          }
          x if x == TokenAliasType::Register as u64 => {
            if let (Some(token_alias), Some(token_type), Some(token_value)) =
              (token_alias, token_type, token_value)
            {
              payload_bytes.put_vi(*token_alias)?;
              payload_bytes.put_vi(*token_type)?;
              payload_bytes.extend_from_slice(token_value);
            }
          }
          x if x == TokenAliasType::UseAlias as u64 => {
            if let Some(token_alias) = token_alias {
              payload_bytes.put_vi(*token_alias)?;
            }
          }
          x if x == TokenAliasType::UseValue as u64 => {
            if let (Some(token_type), Some(token_value)) = (token_type, token_value) {
              payload_bytes.put_vi(*token_type)?;
              payload_bytes.extend_from_slice(token_value);
            }
          }
          _ => {
            return Err(ParseError::KeyValueFormattingError {
              context: "VersionParameter::serialize: unknown alias_type",
            });
          }
        }
        let kvp = KeyValuePair::try_new_bytes(
          VersionSpecificParameterType::AuthorizationToken as u64,
          payload_bytes.freeze(),
        )?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
      VersionParameter::ForwardActionGroup { group_id } => {
        let kvp = KeyValuePair::try_new_varint(
          VersionSpecificParameterType::ForwardActionGroup as u64,
          *group_id,
        )?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
    }
    Ok(bytes.freeze())
  }

  pub fn deserialize(buf: &mut Bytes) -> Result<Self, ParseError> {
    let kvp = KeyValuePair::deserialize(buf)?;
    kvp.try_into()
  }
}

impl TryFrom<KeyValuePair> for VersionParameter {
  type Error = ParseError;
  fn try_from(kvp: KeyValuePair) -> Result<VersionParameter, ParseError> {
    match kvp {
      KeyValuePair::VarInt { type_value, value } => {
        let type_value = VersionSpecificParameterType::try_from(type_value)?;
        match type_value {
          VersionSpecificParameterType::DeliveryTimeout => Ok(VersionParameter::DeliveryTimeout {
            object_timeout: value,
          }),
          VersionSpecificParameterType::MaxCacheDuration => {
            Ok(VersionParameter::MaxCacheDuration { duration: value })
          }
          VersionSpecificParameterType::ForwardActionGroup => {
            Ok(VersionParameter::ForwardActionGroup { group_id: value })
          }
          _ => Err(ParseError::KeyValueFormattingError {
            context: "VersionParameter::deserialize",
          }),
        }
      }
      KeyValuePair::Bytes { type_value, value } => {
        let type_value = VersionSpecificParameterType::try_from(type_value)?;
        let mut payload_bytes = value.clone();
        match type_value {
          VersionSpecificParameterType::AuthorizationToken => {
            let alias_type = payload_bytes.get_vi()?;
            let alias_type = TokenAliasType::try_from(alias_type)?;
            match alias_type {
              TokenAliasType::Delete => {
                let token_alias = payload_bytes.get_vi()?;
                Ok(Self::new_auth_token_delete(token_alias))
              }
              TokenAliasType::Register => {
                let token_alias = payload_bytes.get_vi()?;
                let token_type = payload_bytes.get_vi()?;
                let token_value = payload_bytes.clone();
                Ok(Self::new_auth_token_register(
                  token_alias,
                  token_type,
                  token_value,
                ))
              }
              TokenAliasType::UseAlias => {
                let token_alias = payload_bytes.get_vi()?;
                Ok(Self::new_auth_token_use_alias(token_alias))
              }
              TokenAliasType::UseValue => {
                let token_type = payload_bytes.get_vi()?;
                let token_value = payload_bytes.clone();
                Ok(Self::new_auth_token_use_value(token_type, token_value))
              }
            }
          }
          _ => Err(ParseError::KeyValueFormattingError {
            context: "VersionParameter::deserialize",
          }),
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::VersionParameter;
  use crate::model::common::pair::KeyValuePair;
  use crate::model::common::varint::BufMutVarIntExt;
  use crate::model::parameter::constant::VersionSpecificParameterType;
  use bytes::{Buf, Bytes, BytesMut};

  #[test]
  fn test_roundtrip_max_cache_duration() {
    let orig = VersionParameter::new_max_cache_duration(0x1234);
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let got = VersionParameter::deserialize(&mut buf).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_delivery_timeout() {
    let orig = VersionParameter::new_delivery_timeout(0xABCD);
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let got = VersionParameter::deserialize(&mut buf).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_auth_token_delete() {
    let orig = VersionParameter::new_auth_token_delete(42);
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let got = VersionParameter::deserialize(&mut buf).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_auth_token_register() {
    let orig = VersionParameter::new_auth_token_register(5, 1, Bytes::from_static(b"bytes"));
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let got = VersionParameter::deserialize(&mut buf).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_auth_token_use_alias() {
    let orig = VersionParameter::new_auth_token_use_alias(100);
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let got = VersionParameter::deserialize(&mut buf).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_auth_token_use_value() {
    let orig = VersionParameter::new_auth_token_use_value(16, Bytes::from_static(b"bytes"));
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let got = VersionParameter::deserialize(&mut buf).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_deserialize_invalid_param_type() {
    // Create a KeyValuePair with an invalid type that VersionParameter should reject
    let kvp = KeyValuePair::VarInt {
      type_value: 0xFF_FF, // invalid VersionSpecificParameterType
      value: 0xDEAD,
    };
    let mut buf = kvp.serialize().unwrap();
    let err = VersionParameter::deserialize(&mut buf);
    assert!(err.is_err());
  }

  #[test]
  fn test_deserialize_missing_value_for_cache_duration() {
    let mut buf = BytesMut::new();
    buf
      .put_vi(VersionSpecificParameterType::MaxCacheDuration as u64)
      .unwrap();
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes);
    assert!(err.is_err());
  }

  #[test]
  fn test_deserialize_missing_value_for_delivery_timeout() {
    let mut buf = BytesMut::new();
    buf
      .put_vi(VersionSpecificParameterType::DeliveryTimeout as u64)
      .unwrap();
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes);
    assert!(err.is_err());
  }

  #[test]
  fn test_excess_bytes_after_max_cache_duration() {
    let orig = VersionParameter::new_max_cache_duration(0x55);
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(b"XYZ");
    let mut bytes = buf.freeze();
    let got = VersionParameter::deserialize(&mut bytes).unwrap();
    assert_eq!(got, orig);
    assert_eq!(bytes.remaining(), 3);
    assert_eq!(&bytes[..], b"XYZ");
  }

  #[test]
  fn test_excess_bytes_after_delivery_timeout() {
    let orig = VersionParameter::new_delivery_timeout(0x66);
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(&[1, 2, 3]);
    let mut bytes = buf.freeze();
    let got = VersionParameter::deserialize(&mut bytes).unwrap();
    assert_eq!(got, orig);
    assert_eq!(bytes.remaining(), 3);
    assert_eq!(&bytes[..], &[1, 2, 3]);
  }

  #[test]
  fn test_excess_bytes_after_auth_token_delete() {
    let orig = VersionParameter::new_auth_token_delete(9);
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(b"EXTRA");
    let mut bytes = buf.freeze();
    let got = VersionParameter::deserialize(&mut bytes).unwrap();
    assert_eq!(bytes.remaining(), 5);
    assert_eq!(&bytes[..], b"EXTRA");
  }

  #[test]
  fn test_excess_bytes_after_auth_token_register() {
    let orig = VersionParameter::new_auth_token_register(1, 2, b"x".to_vec().into());
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(b"++");
    let mut bytes = buf.freeze();
    let got = VersionParameter::deserialize(&mut bytes).unwrap();
    assert_eq!(bytes.remaining(), 2);
    assert_eq!(&bytes[..], b"++");
  }

  #[test]
  fn test_excess_bytes_after_auth_token_use_alias() {
    let orig = VersionParameter::new_auth_token_use_alias(3);
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(&[9]);
    let mut bytes = buf.freeze();
    let got = VersionParameter::deserialize(&mut bytes).unwrap();
    assert_eq!(got, orig);
    assert_eq!(bytes.remaining(), 1);
    assert_eq!(&bytes[..], &[9]);
  }

  #[test]
  fn test_excess_bytes_after_auth_token_use_value() {
    let orig = VersionParameter::new_auth_token_use_value(7, b"v".to_vec().into());
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(&[0xAB, 0xCD]);
    let mut bytes = buf.freeze();
    let got = VersionParameter::deserialize(&mut bytes).unwrap();
    assert_eq!(got, orig);
    assert_eq!(bytes.remaining(), 2);
    assert_eq!(&bytes[..], &[0xAB, 0xCD]);
  }
}
