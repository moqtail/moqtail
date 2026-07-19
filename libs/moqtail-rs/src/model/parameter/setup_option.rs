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
  common::pair::KeyValuePair,
  error::ParseError,
  parameter::{authorization_token::AuthorizationToken, constant::SetupOptionType},
};
use bytes::{Bytes, BytesMut};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupOption {
  Path { moqt_path: String },
  AuthorizationToken { token: AuthorizationToken },
  MaxAuthTokenCacheSize { max_size: u64 },
  Authority { authority: String },
  MoqtImplementation { info: String },
}
impl SetupOption {
  pub fn new_path(moqt_path: String) -> Self {
    SetupOption::Path { moqt_path }
  }

  /// Raw-QUIC only: the authority component of the `moqt://` URI.
  /// MUST NOT be sent over WebTransport.
  pub fn new_authority(authority: String) -> Self {
    SetupOption::Authority { authority }
  }

  pub fn new_auth_token(token: AuthorizationToken) -> Self {
    SetupOption::AuthorizationToken { token }
  }

  pub fn new_max_auth_token_cache_size(max_size: u64) -> Self {
    SetupOption::MaxAuthTokenCacheSize { max_size }
  }

  pub fn new_moqt_implementation(info: String) -> Self {
    SetupOption::MoqtImplementation { info }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut bytes = BytesMut::new();
    match self {
      Self::Path { moqt_path } => {
        let data = moqt_path.as_bytes();
        let kvp =
          KeyValuePair::try_new_bytes(SetupOptionType::Path as u64, Bytes::copy_from_slice(data))?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
      Self::AuthorizationToken { token } => match token.serialize() {
        Ok(payload_bytes) => {
          let kvp =
            KeyValuePair::try_new_bytes(SetupOptionType::AuthorizationToken as u64, payload_bytes)?;
          let slice = kvp.serialize()?;
          bytes.extend_from_slice(&slice);
        }
        Err(e) => {
          return Err(ParseError::Other {
            context: "SetupOption::serialize",
            msg: e.to_string(),
          });
        }
      },
      Self::MaxAuthTokenCacheSize { max_size } => {
        let kvp =
          KeyValuePair::try_new_varint(SetupOptionType::MaxAuthTokenCacheSize as u64, *max_size)?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
      Self::MoqtImplementation { info } => {
        let data = info.as_bytes();
        let kvp = KeyValuePair::try_new_bytes(
          SetupOptionType::MoqtImplementation as u64,
          Bytes::copy_from_slice(data),
        )?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
      Self::Authority { authority } => {
        let data = authority.as_bytes();
        let kvp = KeyValuePair::try_new_bytes(
          SetupOptionType::Authority as u64,
          Bytes::copy_from_slice(data),
        )?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
    }
    Ok(bytes.freeze())
  }
  pub fn deserialize(kvp: &KeyValuePair) -> Result<SetupOption, ParseError> {
    match kvp {
      KeyValuePair::VarInt { type_value, value } => {
        let type_value = SetupOptionType::try_from(*type_value)?;
        match type_value {
          SetupOptionType::MaxAuthTokenCacheSize => {
            Ok(SetupOption::MaxAuthTokenCacheSize { max_size: *value })
          }
          _ => Err(ParseError::KeyValueFormattingError {
            context: "SetupOption::deserialize",
          }),
        }
      }
      KeyValuePair::Bytes { type_value, value } => {
        let type_value = SetupOptionType::try_from(*type_value)?;
        let mut payload_bytes = value.clone();
        match type_value {
          SetupOptionType::Path => {
            let moqt_path =
              String::from_utf8(value.to_vec()).map_err(|e| ParseError::InvalidUTF8 {
                context: "SetupOption::deserialize",
                details: e.to_string(),
              })?;
            Ok(SetupOption::Path { moqt_path })
          }
          SetupOptionType::AuthorizationToken => Ok(SetupOption::new_auth_token(
            AuthorizationToken::deserialize(&mut payload_bytes)?,
          )),
          SetupOptionType::MoqtImplementation => {
            let info = String::from_utf8(value.to_vec()).map_err(|e| ParseError::InvalidUTF8 {
              context: "SetupOption::deserialize",
              details: e.to_string(),
            })?;
            Ok(SetupOption::MoqtImplementation { info })
          }
          SetupOptionType::Authority => {
            let authority =
              String::from_utf8(value.to_vec()).map_err(|e| ParseError::InvalidUTF8 {
                context: "SetupOption::deserialize",
                details: e.to_string(),
              })?;
            Ok(SetupOption::Authority { authority })
          }
          _ => Err(ParseError::KeyValueFormattingError {
            context: "SetupOption::deserialize",
          }),
        }
      }
    }
  }
}

impl TryInto<KeyValuePair> for SetupOption {
  type Error = ParseError;
  fn try_into(self) -> Result<KeyValuePair, Self::Error> {
    match self {
      SetupOption::Path { moqt_path } => {
        let data = moqt_path.as_bytes();
        KeyValuePair::try_new_bytes(SetupOptionType::Path as u64, Bytes::copy_from_slice(data))
      }
      SetupOption::AuthorizationToken { token } => KeyValuePair::try_new_bytes(
        SetupOptionType::AuthorizationToken as u64,
        token.serialize()?,
      ),
      SetupOption::MaxAuthTokenCacheSize { max_size } => {
        KeyValuePair::try_new_varint(SetupOptionType::MaxAuthTokenCacheSize as u64, max_size)
      }
      SetupOption::MoqtImplementation { info } => KeyValuePair::try_new_bytes(
        SetupOptionType::MoqtImplementation as u64,
        Bytes::copy_from_slice(info.as_bytes()),
      ),
      SetupOption::Authority { authority } => KeyValuePair::try_new_bytes(
        SetupOptionType::Authority as u64,
        Bytes::copy_from_slice(authority.as_bytes()),
      ),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::SetupOption;
  use crate::model::common::pair::KeyValuePair;
  use crate::model::common::varint::BufMutVarIntExt;
  use crate::model::parameter::constant::SetupOptionType;
  use bytes::{Buf, BytesMut};

  #[test]
  fn test_roundtrip_path() {
    let orig = SetupOption::new_path("test/path".to_string());
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    let got = SetupOption::deserialize(&kvp).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_empty_path() {
    let orig = SetupOption::new_path(String::new());
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    let got = SetupOption::deserialize(&kvp).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_max_auth_cache_size() {
    let orig = SetupOption::new_max_auth_token_cache_size(123456);
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    let got = SetupOption::deserialize(&kvp).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_moqt_implementation() {
    let orig = SetupOption::new_moqt_implementation("moqtail".to_string());
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    let got = SetupOption::deserialize(&kvp).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_authority() {
    let orig = SetupOption::new_authority("example.com:9443".to_string());
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    let got = SetupOption::deserialize(&kvp).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_deserialize_invalid_type() {
    // Create a KeyValuePair with an invalid type that SetupOption should reject
    let kvp = KeyValuePair::VarInt {
      type_value: 999, // invalid SetupOptionType
      value: 42,
    };
    let err = SetupOption::deserialize(&kvp);
    assert!(err.is_err());
  }

  #[test]
  fn test_deserialize_path_missing_length() {
    let mut buf = BytesMut::new();
    buf.put_vi(SetupOptionType::Path as u64).unwrap();
    // no length, no data
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes);
    assert!(err.is_err());
  }

  #[test]
  fn test_deserialize_path_insufficient_data() {
    let mut buf = BytesMut::new();
    buf.put_vi(SetupOptionType::Path as u64).unwrap();
    buf.put_vi(5).unwrap(); // declare 5 bytes
    buf.extend_from_slice(b"abc"); // only 3 bytes
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes);
    assert!(err.is_err());
  }

  #[test]
  fn test_deserialize_max_auth_cache_missing_value() {
    let mut buf = BytesMut::new();
    buf
      .put_vi(SetupOptionType::MaxAuthTokenCacheSize as u64)
      .unwrap();
    // no size varint
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes);
    assert!(err.is_err());
  }

  #[test]
  fn test_excess_bytes_after_path() {
    let orig = SetupOption::new_path("ok".to_string());
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(b"XYZ");
    let mut bytes = buf.freeze();
    let kvp = KeyValuePair::deserialize(&mut bytes).unwrap();
    let got = SetupOption::deserialize(&kvp).unwrap();
    assert_eq!(got, orig);
    assert_eq!(bytes.remaining(), 3);
    assert_eq!(&bytes[..], b"XYZ");
  }

  #[test]
  fn test_excess_bytes_after_max_auth_cache() {
    let orig = SetupOption::new_max_auth_token_cache_size(7);
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(&[1, 2, 3]);
    let mut bytes = buf.freeze();
    let kvp = KeyValuePair::deserialize(&mut bytes).unwrap();
    let got = SetupOption::deserialize(&kvp).unwrap();
    assert_eq!(got, orig);
    assert_eq!(bytes.remaining(), 3);
    assert_eq!(&bytes[..], &[1, 2, 3]);
  }
}
