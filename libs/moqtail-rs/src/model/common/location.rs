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

use bytes::{Bytes, BytesMut};

use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Location {
  pub group: u64,
  pub object: u64,
}

impl Location {
  pub fn new(group: u64, object: u64) -> Self {
    Self { group, object }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(self.group)?;
    buf.put_vi(self.object)?;
    Ok(buf.freeze())
  }

  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let group = bytes.get_vi()?;
    let object = bytes.get_vi()?;
    Ok(Location { group, object })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_roundtrip() {
    let original = Location::new(1, 100);
    let mut bytes = original.serialize().unwrap();
    let deserialized = Location::deserialize(&mut bytes).unwrap();
    assert_eq!(original, deserialized);
  }

  #[test]
  fn test_ordering() {
    let loc1 = Location::new(1, 1);
    let loc2 = Location::new(1, 2);
    let loc3 = Location::new(2, 1);
    assert!(loc1 < loc3);
    assert!(loc2 < loc3);
    assert!(loc1 < loc2);
  }

  #[test]
  fn test_deserialize_error() {
    let mut bytes = BytesMut::new();
    bytes.put_vi(1).unwrap(); // Only group, missing object
    let result = Location::deserialize(&mut bytes.freeze());
    assert!(result.is_err());
  }
}
