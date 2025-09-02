use crate::model::{
  common::pair::KeyValuePair, error::ParseError, parameter::constant::SetupParameterType,
};
use bytes::{Bytes, BytesMut};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupParameter {
  Path { moqt_path: String },
  MaxRequestId { max_id: u64 },
  MaxAuthTokenCacheSize { max_size: u64 },
}
impl SetupParameter {
  pub fn new_path(moqt_path: String) -> Self {
    SetupParameter::Path { moqt_path }
  }

  pub fn new_max_request_id(max_id: u64) -> Self {
    SetupParameter::MaxRequestId { max_id }
  }

  pub fn new_max_auth_token_cache_size(max_size: u64) -> Self {
    SetupParameter::MaxAuthTokenCacheSize { max_size }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut bytes = BytesMut::new();
    match self {
      Self::Path { moqt_path } => {
        let data = moqt_path.as_bytes();
        let kvp = KeyValuePair::try_new_bytes(
          SetupParameterType::Path as u64,
          Bytes::copy_from_slice(data),
        )?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
      Self::MaxRequestId { max_id } => {
        let kvp = KeyValuePair::try_new_varint(SetupParameterType::MaxRequestId as u64, *max_id)?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
      Self::MaxAuthTokenCacheSize { max_size } => {
        let kvp = KeyValuePair::try_new_varint(
          SetupParameterType::MaxAuthTokenCacheSize as u64,
          *max_size,
        )?;
        let slice = kvp.serialize()?;
        bytes.extend_from_slice(&slice);
      }
    }
    Ok(bytes.freeze())
  }
  pub fn deserialize(kvp: &KeyValuePair) -> Result<SetupParameter, ParseError> {
    match kvp {
      KeyValuePair::VarInt { type_value, value } => {
        let type_value = SetupParameterType::try_from(*type_value)?;
        match type_value {
          SetupParameterType::MaxRequestId => Ok(SetupParameter::MaxRequestId { max_id: *value }),
          SetupParameterType::MaxAuthTokenCacheSize => {
            Ok(SetupParameter::MaxAuthTokenCacheSize { max_size: *value })
          }
          _ => Err(ParseError::KeyValueFormattingError {
            context: "SetupParameter::deserialize",
          }),
        }
      }
      KeyValuePair::Bytes { type_value, value } => {
        let type_value = SetupParameterType::try_from(*type_value)?;
        match type_value {
          SetupParameterType::Path => {
            let moqt_path =
              String::from_utf8(value.to_vec()).map_err(|e| ParseError::InvalidUTF8 {
                context: "SetupParameter::deserialize",
                details: e.to_string(),
              })?;
            Ok(SetupParameter::Path { moqt_path })
          }
          _ => Err(ParseError::KeyValueFormattingError {
            context: "SetupParameter::deserialize",
          }),
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::SetupParameter;
  use crate::model::common::pair::KeyValuePair;
  use crate::model::common::varint::BufMutVarIntExt;
  use crate::model::parameter::constant::SetupParameterType;
  use bytes::{Buf, BytesMut};

  #[test]
  fn test_roundtrip_path() {
    let orig = SetupParameter::new_path("test/path".to_string());
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    let got = SetupParameter::deserialize(&kvp).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_empty_path() {
    let orig = SetupParameter::new_path(String::new());
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    let got = SetupParameter::deserialize(&kvp).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_max_request_id() {
    let orig = SetupParameter::new_max_request_id(0xDEAD_BEEFu64);
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    let got = SetupParameter::deserialize(&kvp).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_roundtrip_max_auth_cache_size() {
    let orig = SetupParameter::new_max_auth_token_cache_size(123456);
    let serialized = orig.serialize().unwrap();
    let mut buf = serialized.clone();
    let kvp = KeyValuePair::deserialize(&mut buf).unwrap();
    let got = SetupParameter::deserialize(&kvp).unwrap();
    assert_eq!(orig, got);
    assert_eq!(buf.remaining(), 0);
  }

  #[test]
  fn test_deserialize_invalid_type() {
    // Create a KeyValuePair with an invalid type that SetupParameter should reject
    let kvp = KeyValuePair::VarInt {
      type_value: 999, // invalid SetupParameterType
      value: 42,
    };
    let err = SetupParameter::deserialize(&kvp);
    assert!(err.is_err());
  }

  #[test]
  fn test_deserialize_path_missing_length() {
    let mut buf = BytesMut::new();
    buf.put_vi(SetupParameterType::Path as u64).unwrap();
    // no length, no data
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes);
    assert!(err.is_err());
  }

  #[test]
  fn test_deserialize_path_insufficient_data() {
    let mut buf = BytesMut::new();
    buf.put_vi(SetupParameterType::Path as u64).unwrap();
    buf.put_vi(5).unwrap(); // declare 5 bytes
    buf.extend_from_slice(b"abc"); // only 3 bytes
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes);
    assert!(err.is_err());
  }

  #[test]
  fn test_deserialize_max_request_id_missing_value() {
    let mut buf = BytesMut::new();
    buf.put_vi(SetupParameterType::MaxRequestId as u64).unwrap();
    // no ID varint
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes);
    assert!(err.is_err());
  }

  #[test]
  fn test_deserialize_max_auth_cache_missing_value() {
    let mut buf = BytesMut::new();
    buf
      .put_vi(SetupParameterType::MaxAuthTokenCacheSize as u64)
      .unwrap();
    // no size varint
    let mut bytes = buf.freeze();
    let err = KeyValuePair::deserialize(&mut bytes);
    assert!(err.is_err());
  }

  #[test]
  fn test_excess_bytes_after_path() {
    let orig = SetupParameter::new_path("ok".to_string());
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(b"XYZ");
    let mut bytes = buf.freeze();
    let kvp = KeyValuePair::deserialize(&mut bytes).unwrap();
    let got = SetupParameter::deserialize(&kvp).unwrap();
    assert_eq!(got, orig);
    assert_eq!(bytes.remaining(), 3);
    assert_eq!(&bytes[..], b"XYZ");
  }

  #[test]
  fn test_excess_bytes_after_max_request_id() {
    let orig = SetupParameter::new_max_request_id(42);
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(&[0xFF, 0xEE]);
    let mut bytes = buf.freeze();
    let kvp = KeyValuePair::deserialize(&mut bytes).unwrap();
    let got = SetupParameter::deserialize(&kvp).unwrap();
    assert_eq!(got, orig);
    assert_eq!(bytes.remaining(), 2);
    assert_eq!(&bytes[..], &[0xFF, 0xEE]);
  }

  #[test]
  fn test_excess_bytes_after_max_auth_cache() {
    let orig = SetupParameter::new_max_auth_token_cache_size(7);
    let mut buf = BytesMut::from(&orig.serialize().unwrap()[..]);
    buf.extend_from_slice(&[1, 2, 3]);
    let mut bytes = buf.freeze();
    let kvp = KeyValuePair::deserialize(&mut bytes).unwrap();
    let got = SetupParameter::deserialize(&kvp).unwrap();
    assert_eq!(got, orig);
    assert_eq!(bytes.remaining(), 3);
    assert_eq!(&bytes[..], &[1, 2, 3]);
  }
}
