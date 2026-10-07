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

use crate::model::common::pair::KeyValuePair;
use crate::model::error::ParseError;
use bytes::Bytes;

/// MoQT object extension Type for FEC Source Info (draft-ramadan-moq-fec).
///
/// The value is **odd** on purpose. Under the MoQT key-value parity rule an even
/// Type carries a single varint Value, so a length-prefixed even Type desyncs any
/// conformant parser or re-encoding relay on the first FEC-tagged object. Because
/// `0x11` is odd, [`KeyValuePair`] already reads it as a length-prefixed byte
/// string and this extension needs no parser special case.
pub const FEC_SOURCE_INFO_TYPE: u64 = 0x11;

/// FEC Source Info Value length. Exactly this; any other length is an error.
pub const FEC_SOURCE_INFO_LEN: usize = 4;

/// FEC Source Info object extension (draft-ramadan-moq-fec).
///
/// Value layout: `SS_ID(u32 BE)`, exactly [`FEC_SOURCE_INFO_LEN`] bytes. SS_ID is the
/// source symbol's sequence number — the same value and definition the Source FEC
/// Payload ID trailer carries on the MMTP path, so one identifier covers both paths.
///
/// The field is **fixed width, not a QUIC varint**, for two reasons. MoQT permits
/// non-minimal varint encodings, so a varint would vary the extension's own length
/// without varying its meaning. And the varint encoding is a property of the session,
/// which MoQT redefined at draft-17, so a varint would decode differently on two drafts.
///
/// The byte layout is pinned by `vectors/fec-source-info/cases.json`, which is shared
/// with libmmt so both engines are checked against the same hand-authored bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FecSourceInfo {
  /// Source symbol sequence number.
  pub ss_id: u32,
}

impl FecSourceInfo {
  pub fn new(ss_id: u32) -> Self {
    Self { ss_id }
  }

  /// Encodes the extension Value bytes.
  pub fn encode(&self) -> Bytes {
    Bytes::copy_from_slice(&self.ss_id.to_be_bytes())
  }

  /// Decodes the extension Value bytes. Fails closed on any length other than
  /// [`FEC_SOURCE_INFO_LEN`].
  pub fn decode(value: &[u8]) -> Result<Self, ParseError> {
    let bytes: [u8; FEC_SOURCE_INFO_LEN] =
      value
        .try_into()
        .map_err(|_| ParseError::ProtocolViolation {
          context: "FecSourceInfo::decode",
          details: format!(
            "Value must be {FEC_SOURCE_INFO_LEN} bytes, got {}",
            value.len()
          ),
        })?;
    Ok(Self {
      ss_id: u32::from_be_bytes(bytes),
    })
  }

  /// Wraps this extension as an object property, ready for any object's property list.
  pub fn to_key_value_pair(&self) -> Result<KeyValuePair, ParseError> {
    KeyValuePair::try_new_bytes(FEC_SOURCE_INFO_TYPE, self.encode())
  }

  /// Decodes an object property known to be FEC Source Info.
  ///
  /// Returns [`ParseError::ProtocolViolation`] if the pair is not
  /// [`FEC_SOURCE_INFO_TYPE`] or does not carry a byte-string Value.
  pub fn from_key_value_pair(pair: &KeyValuePair) -> Result<Self, ParseError> {
    match pair {
      KeyValuePair::Bytes { type_value, value } if *type_value == FEC_SOURCE_INFO_TYPE => {
        Self::decode(value)
      }
      KeyValuePair::Bytes { type_value, .. } | KeyValuePair::VarInt { type_value, .. } => {
        Err(ParseError::ProtocolViolation {
          context: "FecSourceInfo::from_key_value_pair",
          details: if *type_value == FEC_SOURCE_INFO_TYPE {
            "FEC Source Info Value must be a byte string".to_string()
          } else {
            format!("expected type {FEC_SOURCE_INFO_TYPE}, got {type_value}")
          },
        })
      }
    }
  }

  /// Finds the FEC Source Info extension in an object's property list, or returns
  /// `Ok(None)` when the object carries none. Use on `SubgroupObject`, `Datagram`
  /// and `FetchObject`, each of which carries `properties` as a plain KVP list.
  pub fn from_properties(properties: &[KeyValuePair]) -> Result<Option<Self>, ParseError> {
    properties
      .iter()
      .find(|p| match p {
        KeyValuePair::Bytes { type_value, .. } | KeyValuePair::VarInt { type_value, .. } => {
          *type_value == FEC_SOURCE_INFO_TYPE
        }
      })
      .map(Self::from_key_value_pair)
      .transpose()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde::Deserialize;

  #[derive(Deserialize)]
  #[serde(rename_all = "camelCase")]
  struct Case {
    name: String,
    ss_id: u32,
    value: String,
  }

  #[derive(Deserialize)]
  #[serde(rename_all = "camelCase")]
  struct Vectors {
    extension_type: u64,
    value_length: usize,
    cases: Vec<Case>,
    invalid_value_lengths: Vec<usize>,
  }

  /// The vectors are hand-authored from the specification and shared verbatim with
  /// libmmt, so both engines are checked against the same bytes rather than against
  /// each other's round trip.
  fn vectors() -> Vectors {
    let raw = include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../vectors/fec-source-info/cases.json"
    ));
    serde_json::from_str(raw).expect("fec-source-info vectors parse")
  }

  fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
  }

  fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
      .step_by(2)
      .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
      .collect()
  }

  #[test]
  fn codepoint_matches_the_shared_vectors() {
    assert_eq!(FEC_SOURCE_INFO_TYPE, vectors().extension_type);
  }

  #[test]
  fn codepoint_is_odd_so_parity_alone_gives_a_byte_string_value() {
    assert_eq!(FEC_SOURCE_INFO_TYPE % 2, 1);
  }

  #[test]
  fn value_length_matches_the_shared_vectors() {
    assert_eq!(FEC_SOURCE_INFO_LEN, vectors().value_length);
  }

  #[test]
  fn encodes_every_case_to_the_pinned_bytes() {
    for c in vectors().cases {
      assert_eq!(
        hex(&FecSourceInfo::new(c.ss_id).encode()),
        c.value,
        "case {}",
        c.name
      );
    }
  }

  #[test]
  fn decodes_every_case_back_to_its_ss_id() {
    for c in vectors().cases {
      let decoded =
        FecSourceInfo::decode(&unhex(&c.value)).unwrap_or_else(|e| panic!("case {}: {e}", c.name));
      assert_eq!(decoded, FecSourceInfo::new(c.ss_id), "case {}", c.name);
    }
  }

  #[test]
  fn round_trips_every_case_through_an_object_property_list() {
    for c in vectors().cases {
      let info = FecSourceInfo::new(c.ss_id);
      let properties = vec![
        KeyValuePair::try_new_varint(0, 10).unwrap(),
        info.to_key_value_pair().unwrap(),
      ];
      assert_eq!(
        FecSourceInfo::from_properties(&properties).unwrap(),
        Some(info),
        "case {}",
        c.name
      );
    }
  }

  /// 8 and 12 were the SBN/ESI[/ObjectLength] widths of the retired format version 1.
  /// They are refused, not decoded: there is no compatibility shim.
  #[test]
  fn refuses_every_invalid_value_length() {
    for len in vectors().invalid_value_lengths {
      assert!(
        FecSourceInfo::decode(&vec![0u8; len]).is_err(),
        "a {len}-byte Value must be refused"
      );
    }
  }

  #[test]
  fn survives_a_wire_round_trip_through_the_generic_kvp_codec() {
    let info = FecSourceInfo::new(1234);
    let mut wire = info.to_key_value_pair().unwrap().serialize().unwrap();
    let parsed = KeyValuePair::deserialize(&mut wire).unwrap();
    assert_eq!(FecSourceInfo::from_key_value_pair(&parsed).unwrap(), info);
  }

  #[test]
  fn reports_no_extension_when_the_object_carries_none() {
    assert_eq!(FecSourceInfo::from_properties(&[]).unwrap(), None);
    let other = vec![KeyValuePair::try_new_varint(0, 10).unwrap()];
    assert_eq!(FecSourceInfo::from_properties(&other).unwrap(), None);
  }

  #[test]
  fn refuses_a_pair_of_the_wrong_type() {
    let pair = KeyValuePair::try_new_bytes(0x13, Bytes::from_static(&[0u8; 4])).unwrap();
    assert!(FecSourceInfo::from_key_value_pair(&pair).is_err());
  }
}
