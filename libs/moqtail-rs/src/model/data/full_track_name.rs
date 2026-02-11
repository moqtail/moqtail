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

use std::fmt::{Debug, Display, Formatter};

use bytes::{Buf, Bytes};

use crate::model::common::tuple::{Tuple, TupleField};
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;

const MAX_NAMESPACE_TUPLE_COUNT: usize = 32;
const MAX_FULL_TRACK_NAME_LENGTH: usize = 4096;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FullTrackName {
  pub namespace: Tuple,
  pub name: TupleField,
}

impl Debug for FullTrackName {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self)
  }
}

impl Display for FullTrackName {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    let mut res = String::new();
    for (i, field) in self.namespace.fields.iter().enumerate() {
      if i > 0 {
        res.push('-');
      }
      res.push_str(&field.to_string());
    }
    res.push_str("--");
    res.push_str(&self.name.to_string());
    write!(f, "{}", res)
  }
}

impl FullTrackName {
  pub fn new(namespace: Tuple, name: TupleField) -> Result<Self, ParseError> {
    let ns_count = namespace.fields.len();
    if ns_count == 0 || ns_count > MAX_NAMESPACE_TUPLE_COUNT {
      return Err(ParseError::TrackNameError {
        context: "FullTrackName::new(ns_count)",
        details: format!(
          "Namespace cannot be empty or cannot exceed {MAX_NAMESPACE_TUPLE_COUNT} fields"
        ),
      });
    }

    let total_len = Self::namespace_length(&namespace)? + name.len();
    if total_len > MAX_FULL_TRACK_NAME_LENGTH {
      return Err(ParseError::TrackNameError {
        context: "FullTrackName::new(total_length)",
        details: format!("Total length cannot exceed {MAX_FULL_TRACK_NAME_LENGTH}"),
      });
    }
    Ok(Self { namespace, name })
  }

  fn namespace_length(namespace: &Tuple) -> Result<usize, ParseError> {
    Ok(namespace.serialize()?.len())
  }

  pub fn try_new(namespace: &str, name: &str) -> Result<Self, ParseError> {
    let namespace_bytes = Tuple::from_utf8_path(namespace);
    let name_bytes = TupleField::from_utf8(name);

    Self::new(namespace_bytes, name_bytes)
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = bytes::BytesMut::new();
    buf.extend(self.namespace.serialize());
    buf.put_vi(self.name.len())?;
    buf.extend_from_slice(self.name.as_bytes());
    Ok(buf.freeze())
  }

  pub fn deserialize(buf: &mut Bytes) -> Result<Self, ParseError> {
    let namespace = Tuple::deserialize(buf)?;
    let ns_count = namespace.fields.len();
    if ns_count == 0 || ns_count > MAX_NAMESPACE_TUPLE_COUNT {
      return Err(ParseError::TrackNameError {
        context: "FullTrackName::deserialize(ns_count)",
        details: format!(
          "Namespace cannot be empty or cannot exceed {MAX_NAMESPACE_TUPLE_COUNT} fields"
        ),
      });
    }
    let name_len = buf.get_vi()? as usize;

    if buf.remaining() < name_len {
      return Err(ParseError::NotEnoughBytes {
        context: "FullTrackName::deserialize(name_length)",
        needed: name_len,
        available: buf.remaining(),
      });
    }
    let name = buf.copy_to_bytes(name_len);

    let total_len = Self::namespace_length(&namespace)? + name.len();
    if total_len > MAX_FULL_TRACK_NAME_LENGTH {
      return Err(ParseError::TrackNameError {
        context: "FullTrackName::deserialize(total_length)",
        details: format!("Total length cannot exceed {MAX_FULL_TRACK_NAME_LENGTH}"),
      });
    }

    Ok(Self {
      namespace,
      name: TupleField::new(name),
    })
  }
}
