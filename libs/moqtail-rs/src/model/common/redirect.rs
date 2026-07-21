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

use crate::model::common::tuple::{Tuple, TupleField};
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, Bytes, BytesMut};

/// Directs the peer to retry a request at a different URI and/or Full Track Name
/// (draft §10.6.1). Carried by a REQUEST_ERROR whose code is REDIRECT.
#[derive(Debug, PartialEq, Clone)]
pub struct Redirect {
  /// The URI to connect to. Empty means reuse the current session's URI.
  pub connect_uri: Option<String>,
  /// Track Namespace for the redirected request; empty (with an empty name)
  /// means reuse the original request's values.
  pub track_namespace: Tuple,
  /// Track Name for the redirected request; empty for namespace-scoped requests.
  pub track_name: TupleField,
}

impl Redirect {
  pub fn new(connect_uri: Option<String>, track_namespace: Tuple, track_name: TupleField) -> Self {
    let connect_uri = connect_uri.filter(|uri| !uri.is_empty());
    Self {
      connect_uri,
      track_namespace,
      track_name,
    }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    match &self.connect_uri {
      Some(uri) => {
        buf.put_vi(uri.len())?;
        buf.extend_from_slice(uri.as_bytes());
      }
      None => buf.put_vi(0)?,
    }
    buf.extend_from_slice(&self.track_namespace.serialize()?);
    buf.put_vi(self.track_name.len())?;
    buf.extend_from_slice(self.track_name.as_bytes());
    Ok(buf.freeze())
  }

  pub fn deserialize(payload: &mut Bytes) -> Result<Self, ParseError> {
    let uri_len: usize = payload
      .get_vi()?
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "Redirect::deserialize(uri_len)",
        from_type: "u64",
        to_type: "usize",
        details: e.to_string(),
      })?;

    let connect_uri = if uri_len == 0 {
      None
    } else {
      if payload.remaining() < uri_len {
        return Err(ParseError::NotEnoughBytes {
          context: "Redirect::deserialize(connect_uri)",
          needed: uri_len,
          available: payload.remaining(),
        });
      }
      let bytes = payload.copy_to_bytes(uri_len);
      Some(
        String::from_utf8(bytes.to_vec()).map_err(|e| ParseError::InvalidUTF8 {
          context: "Redirect::deserialize(connect_uri)",
          details: e.to_string(),
        })?,
      )
    };

    let track_namespace = Tuple::deserialize(payload)?;

    let name_len: usize =
      payload
        .get_vi()?
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "Redirect::deserialize(name_len)",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;
    if payload.remaining() < name_len {
      return Err(ParseError::NotEnoughBytes {
        context: "Redirect::deserialize(track_name)",
        needed: name_len,
        available: payload.remaining(),
      });
    }
    let track_name = TupleField::new(payload.copy_to_bytes(name_len));

    Ok(Self {
      connect_uri,
      track_namespace,
      track_name,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn round_trips_with_uri_and_track() {
    let r = Redirect::new(
      Some("moqt://other.example".to_string()),
      Tuple::from_utf8_path("room1/audio"),
      TupleField::from_utf8("track-9"),
    );
    let mut bytes = r.serialize().unwrap();
    assert_eq!(Redirect::deserialize(&mut bytes).unwrap(), r);
    assert!(!bytes.has_remaining());
  }

  #[test]
  fn round_trips_empty_uri_and_empty_track() {
    let r = Redirect::new(None, Tuple::new(), TupleField::from_utf8(""));
    assert_eq!(r.connect_uri, None);
    let mut bytes = r.serialize().unwrap();
    assert_eq!(Redirect::deserialize(&mut bytes).unwrap(), r);
    assert!(!bytes.has_remaining());
  }
}
