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

use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, Bytes, BytesMut};
use core::hash::{Hash, Hasher};
use std::fmt::{Debug, Display};

#[derive(Clone, Default, PartialEq, Eq)]
pub struct TupleField {
  value: Bytes,
}

impl Hash for TupleField {
  fn hash<H: Hasher>(&self, state: &mut H) {
    self.value.hash(state);
  }
}

impl Debug for TupleField {
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    write!(f, "{}", self.as_str())
  }
}

impl Display for TupleField {
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    write!(f, "{}", self.as_str())
  }
}

impl TupleField {
  pub fn new(value: Bytes) -> Self {
    Self { value }
  }

  pub fn len(&self) -> usize {
    self.value.len()
  }

  pub fn is_empty(&self) -> bool {
    self.value.is_empty()
  }

  pub fn from_utf8(path: &str) -> Self {
    Self {
      value: Bytes::copy_from_slice(path.as_bytes()),
    }
  }

  pub fn as_str(&self) -> String {
    let mut res = String::new();
    for &b in self.value.as_ref() {
      match b {
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => {
          res.push(b as char);
        }
        _ => {
          res.push_str(&format!(".{:02x}", b));
        }
      }
    }
    res
  }

  pub fn as_bytes(&self) -> &[u8] {
    &self.value
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(self.value.len())?;
    buf.extend_from_slice(&self.value);
    Ok(buf.freeze())
  }

  pub fn deserialize(buf: &mut Bytes) -> Result<Self, ParseError> {
    let length = buf.get_vi()?;

    let length_usize: usize =
      length
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "TupleField::decode",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;

    if buf.remaining() < length_usize {
      return Err(ParseError::NotEnoughBytes {
        context: "TupleField::decode",
        needed: length_usize,
        available: buf.remaining(),
      });
    }
    let bytes = buf.copy_to_bytes(length_usize);

    Ok(Self { value: bytes })
  }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tuple {
  pub fields: Vec<TupleField>,
}

impl Hash for Tuple {
  fn hash<H: Hasher>(&self, state: &mut H) {
    self.fields.hash(state);
  }
}

impl Tuple {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn from_utf8_path(path: &str) -> Self {
    let mut tuple = Tuple::new();

    for part in path.split('/') {
      if part.is_empty() {
        continue;
      }

      tuple.add(TupleField::from_utf8(part));
    }
    tuple
  }

  pub fn to_utf8_path(&self) -> String {
    let mut path = String::new();

    for field in &self.fields {
      path.push('/');
      path.push_str(&String::from_utf8_lossy(&field.value));
    }

    path
  }
  pub fn add(&mut self, field: TupleField) {
    self.fields.push(field);
  }

  pub fn get(&self, index: usize) -> &TupleField {
    &self.fields[index]
  }

  pub fn set(&mut self, index: usize, f: TupleField) {
    self.fields[index] = f;
  }

  pub fn clear(&mut self) {
    self.fields.clear();
  }

  pub fn starts_with(&self, parent: &Tuple) -> bool {
    // parent cannot have more fields
    if parent.fields.len() > self.fields.len() {
      return false;
    }

    // if any of my fields is not equal
    // I'm not a child
    for i in 0..parent.fields.len() {
      if self.fields[i].ne(&parent.fields[i]) {
        return false;
      }
    }

    // it seems I'm a  child :)
    true
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(self.fields.len() as u64)?;

    for field in &self.fields {
      buf.extend_from_slice(&field.serialize()?);
    }

    Ok(buf.freeze())
  }
  pub fn deserialize(buf: &mut Bytes) -> Result<Self, ParseError> {
    let count = buf.get_vi()?;

    let count_usize: usize =
      count
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "Tuple::decode",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;

    let mut fields = Vec::new();
    for _ in 0..count_usize {
      let field = TupleField::deserialize(buf)?;
      fields.push(field);
    }

    Ok(Self { fields })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  pub fn test_tuple_field() {
    let field = TupleField::from_utf8("hello");
    assert_eq!(field.value, Bytes::from_static(b"hello"));

    // add arbitrary bytes at the end of the byte stream of the field
    let mut buf = BytesMut::new();
    buf.extend(field.serialize().unwrap());
    buf.extend(b"arbitrary bytes");
    // now decode this and see whether it still gets the correct value
    let mut buf = buf.freeze();
    let decoded_field = TupleField::deserialize(&mut buf).expect("Decoding TupleField failed");

    assert_eq!(decoded_field.value, Bytes::from_static(b"hello"));
    assert_eq!(buf, Bytes::from_static(b"arbitrary bytes")); // Check remaining buffer
  }

  #[test]
  fn test_tuple_field_not_enough_bytes_for_value() {
    let mut buf = BytesMut::new();
    buf.put_vi(10).unwrap(); // Length = 10
    buf.extend_from_slice(b"short"); // Only 5 bytes available
    let mut bytes = buf.freeze();
    let result = TupleField::deserialize(&mut bytes);
    assert!(matches!(result, Err(ParseError::NotEnoughBytes { .. })));
  }

  #[test]
  fn test_tuple() {
    let mut tuple = Tuple::new();
    tuple.add(TupleField::from_utf8("hello"));
    tuple.add(TupleField::from_utf8("world"));
    assert_eq!(tuple.fields.len(), 2);
    assert_eq!(tuple.get(0).value, Bytes::from_static(b"hello"));
    assert_eq!(tuple.get(1).value, Bytes::from_static(b"world"));
    assert_eq!(tuple.to_utf8_path(), "/hello/world");

    // Test roundtrip
    let mut bytes = tuple.serialize().unwrap();
    let decoded_tuple = Tuple::deserialize(&mut bytes).expect("Decoding Tuple failed");
    assert_eq!(tuple, decoded_tuple);
    assert!(bytes.is_empty()); // Ensure all bytes were consumed
  }

  #[test]
  fn test_tuple_decode_not_enough_bytes_for_field() {
    let mut buf = BytesMut::new();
    buf.put_vi(2).unwrap(); // Count = 2 fields
    // First field (correct)
    buf.put_vi(5).unwrap();
    buf.extend_from_slice(b"hello");
    // Second field (length only, no value)
    buf.put_vi(5).unwrap();
    let mut bytes = buf.freeze();
    let result = Tuple::deserialize(&mut bytes);
    assert!(result.is_err())
  }

  #[test]
  fn test_equality() {
    let mut tuple1 = Tuple::new();
    tuple1.add(TupleField::from_utf8("hello"));
    tuple1.add(TupleField::from_utf8("world"));

    let mut tuple2 = Tuple::new();
    tuple2.add(TupleField::from_utf8("hello"));
    tuple2.add(TupleField::from_utf8("world"));

    assert_eq!(tuple1, tuple2);
  }

  #[test]
  fn test_from_utf8_path() {
    let tuple = Tuple::from_utf8_path("/this/is/a/very/long/path");
    assert_eq!(tuple.fields.len(), 6);
    assert_eq!(tuple.to_utf8_path(), "/this/is/a/very/long/path");
  }
}
