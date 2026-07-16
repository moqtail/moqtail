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

//! Loads the shared conformance fixtures in `dev/conformance/draft18/`.
//!
//! The fixtures are normative for both this crate and `moqtail-ts`; see the README
//! there. This module is the Rust half of "codepoints live in one place": the values
//! are never repeated here, only read.
//!
//! Fixtures are embedded with `include_str!`, so they are resolved relative to this
//! source file at compile time and no test depends on the working directory.

use serde::Deserialize;

/// The key this crate is listed under in a fixture's `pending` markers.
pub const LANG: &str = "rs";

macro_rules! fixture {
  ($name:literal) => {
    include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../dev/conformance/draft18/",
      $name
    ))
  };
}

/// One codepoint in a fixture: a name, a value, and whether a stack is exempt yet.
#[derive(Debug, Deserialize)]
pub struct Entry {
  pub name: String,
  /// Hex string, e.g. `"0x50"`. Use [`Entry::codepoint`] rather than parsing it again.
  pub value: String,
  #[serde(default)]
  pub reserved: bool,
  #[serde(default)]
  pub pending: std::collections::HashMap<String, u64>,
  #[serde(default)]
  pub notes: Option<String>,
  #[serde(default)]
  pub stream: Option<String>,
  #[serde(default)]
  pub scope: Option<String>,
  #[serde(default)]
  pub spec: Option<String>,
}

impl Entry {
  /// The codepoint as a number.
  pub fn codepoint(&self) -> u64 {
    parse_hex(&self.value)
  }

  /// The issue that will make this crate conform, or `None` if it must conform now.
  pub fn pending_issue(&self) -> Option<u64> {
    self.pending.get(LANG).copied()
  }
}

/// Parses a `"0x2F00"`-style fixture value.
pub fn parse_hex(s: &str) -> u64 {
  let stripped = s
    .strip_prefix("0x")
    .or_else(|| s.strip_prefix("0X"))
    .unwrap_or_else(|| panic!("fixture value {s:?} must be a 0x-prefixed hex string"));
  u64::from_str_radix(stripped, 16).unwrap_or_else(|e| panic!("fixture value {s:?}: {e}"))
}

/// A registry: the draft's table, plus what each stack still gets wrong.
#[derive(Debug, Deserialize)]
pub struct Registry {
  pub source: String,
  pub entries: Vec<Entry>,
  /// Codepoints a stack still defines that the draft does not.
  #[serde(default)]
  pub not_in_draft: Vec<Entry>,
  /// Deliberate, permanent divergence. Never asserted against the draft.
  #[serde(default)]
  pub local_extensions: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct ParameterTypes {
  pub setup_options: Registry,
  pub message_parameters: Registry,
}

/// `property_types.json`: the draft's own table, plus the provisional registrations
/// other moq drafts hold in the same number space.
#[derive(Debug, Deserialize)]
pub struct PropertyTypes {
  #[serde(flatten)]
  pub registry: Registry,
  pub provisional: Registry,
}

pub fn message_types() -> Registry {
  parse(fixture!("message_types.json"), "message_types.json")
}

pub fn request_error_codes() -> Registry {
  parse(
    fixture!("request_error_codes.json"),
    "request_error_codes.json",
  )
}

pub fn termination_codes() -> Registry {
  parse(fixture!("termination_codes.json"), "termination_codes.json")
}

pub fn stream_reset_codes() -> Registry {
  parse(
    fixture!("stream_reset_codes.json"),
    "stream_reset_codes.json",
  )
}

pub fn property_types() -> PropertyTypes {
  parse(fixture!("property_types.json"), "property_types.json")
}

pub fn parameter_types() -> ParameterTypes {
  parse(fixture!("parameter_types.json"), "parameter_types.json")
}

pub fn varint() -> Varint {
  parse(fixture!("varint.json"), "varint.json")
}

fn parse<T: serde::de::DeserializeOwned>(raw: &str, name: &str) -> T {
  serde_json::from_str(raw).unwrap_or_else(|e| panic!("parsing fixture {name}: {e}"))
}

// -- varint.json ------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct Varint {
  pub vectors: Section<Vector>,
  pub boundaries: Section<Boundary>,
  pub non_minimal: Section<NonMinimal>,
  pub prefix_shapes: Section<PrefixShape>,
  pub roundtrip_values: Section<String>,
  pub truncated: Section<Truncated>,
}

#[derive(Debug, Deserialize)]
pub struct Section<T> {
  pub entries: Vec<T>,
}

#[derive(Debug, Deserialize)]
pub struct Vector {
  pub encoding: String,
  pub value: String,
  pub minimal: bool,
}

#[derive(Debug, Deserialize)]
pub struct Boundary {
  pub value: String,
  pub length: usize,
  pub encoding: String,
}

#[derive(Debug, Deserialize)]
pub struct NonMinimal {
  pub encoding: String,
  pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct PrefixShape {
  pub value: String,
  pub mask: String,
  pub prefix: String,
}

#[derive(Debug, Deserialize)]
pub struct Truncated {
  pub encoding: String,
  pub reason: String,
}

/// Parses a decimal fixture value. Varint values are strings because 2^64-1 does not
/// survive a JSON number.
pub fn parse_u64(s: &str) -> u64 {
  s.parse()
    .unwrap_or_else(|e| panic!("fixture value {s:?}: {e}"))
}

/// Decodes a lowercase-hex fixture byte sequence.
pub fn parse_bytes(hex: &str) -> Vec<u8> {
  assert!(
    hex.len().is_multiple_of(2),
    "fixture encoding {hex:?} has an odd number of hex digits"
  );
  (0..hex.len())
    .step_by(2)
    .map(|i| {
      u8::from_str_radix(&hex[i..i + 2], 16)
        .unwrap_or_else(|e| panic!("fixture encoding {hex:?}: {e}"))
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_fixture_loads() {
    assert!(!message_types().entries.is_empty());
    assert!(!request_error_codes().entries.is_empty());
    assert!(!termination_codes().entries.is_empty());
    assert!(!stream_reset_codes().entries.is_empty());
    assert!(!property_types().registry.entries.is_empty());
    assert!(!property_types().provisional.entries.is_empty());
    assert!(!parameter_types().setup_options.entries.is_empty());
    assert!(!parameter_types().message_parameters.entries.is_empty());
    assert!(!varint().vectors.entries.is_empty());
  }

  /// A fixture-driven test that silently iterates nothing passes while asserting
  /// nothing, so every section the varint tests walk must be non-empty.
  #[test]
  fn no_varint_section_is_empty() {
    let v = varint();
    assert!(!v.vectors.entries.is_empty(), "vectors");
    assert!(!v.boundaries.entries.is_empty(), "boundaries");
    assert!(!v.non_minimal.entries.is_empty(), "non_minimal");
    assert!(!v.prefix_shapes.entries.is_empty(), "prefix_shapes");
    assert!(!v.roundtrip_values.entries.is_empty(), "roundtrip_values");
    assert!(!v.truncated.entries.is_empty(), "truncated");
  }

  #[test]
  fn parses_hex_and_decimal_fixture_values() {
    assert_eq!(parse_hex("0x2F00"), 0x2F00);
    assert_eq!(parse_hex("0x0"), 0);
    assert_eq!(parse_bytes("ff0100"), vec![0xff, 0x01, 0x00]);
    assert_eq!(parse_bytes(""), Vec::<u8>::new());
    // The value that cannot round-trip through a JSON number.
    assert_eq!(parse_u64("18446744073709551615"), u64::MAX);
  }
}

/// Asserts this crate's enums against the fixtures.
///
/// Nothing here repeats a codepoint: each test asks the enum what it parses a fixture
/// value as, and compares that against the fixture. Adding a variant is therefore
/// picked up automatically — including by the pending checks, which fail once a
/// marked entry starts conforming.
#[cfg(test)]
mod enum_conformance {
  use super::*;
  use crate::model::control::constant::{ControlMessageType, RequestErrorCode};
  use crate::model::error::TerminationCode;
  use crate::model::parameter::constant::{MessageParameterType, SetupOptionType};
  use crate::model::property::constant::{LOCPropertyId, TrackPropertyType};

  /// `SUBSCRIBE_NAMESPACE` -> `SubscribeNamespace`, this crate's identifier convention.
  fn pascal(name: &str) -> String {
    name
      .split('_')
      .map(|part| {
        let mut chars = part.chars();
        match chars.next() {
          Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
          None => String::new(),
        }
      })
      .collect()
  }

  /// Spec names whose Rust identifier is not the plain PascalCase of the name.
  fn ident_of(entry: &Entry, exceptions: &[(&str, &str)]) -> String {
    exceptions
      .iter()
      .find(|(spec_name, _)| *spec_name == entry.name)
      .map(|(_, ident)| (*ident).to_string())
      .unwrap_or_else(|| pascal(&entry.name))
  }

  /// `name_of` reports what this crate parses a codepoint as, or `None` if it rejects it.
  fn assert_registry(
    reg: &Registry,
    exceptions: &[(&str, &str)],
    name_of: impl Fn(u64) -> Option<String>,
  ) {
    for entry in &reg.entries {
      let actual = name_of(entry.codepoint());
      let expected = ident_of(entry, exceptions);
      let conforms = if entry.reserved {
        actual.is_none()
      } else {
        actual.as_deref() == Some(expected.as_str())
      };

      match entry.pending_issue() {
        Some(issue) => assert!(
          !conforms,
          "{} ({}) is marked pending{{rs:{}}} but now conforms — delete the marker in the fixture ({})",
          entry.name, entry.value, issue, reg.source
        ),
        None if entry.reserved => assert!(
          actual.is_none(),
          "{} ({}) is RESERVED and must be rejected, but parsed as {:?} ({})",
          entry.name,
          entry.value,
          actual,
          reg.source
        ),
        None => assert_eq!(
          actual.as_deref(),
          Some(expected.as_str()),
          "{} ({}) does not match the fixture ({})",
          entry.name,
          entry.value,
          reg.source
        ),
      }
    }

    for entry in &reg.not_in_draft {
      let actual = name_of(entry.codepoint());
      match entry.pending_issue() {
        Some(issue) => assert!(
          actual.is_some(),
          "{} ({}) is listed in not_in_draft pending{{rs:{}}} but is already gone — delete the entry ({})",
          entry.name,
          entry.value,
          issue,
          reg.source
        ),
        None => assert!(
          actual.is_none(),
          "{} ({}) is not in the draft and must be rejected, but parsed as {:?} ({})",
          entry.name,
          entry.value,
          actual,
          reg.source
        ),
      }
    }
  }

  #[test]
  fn control_message_types_match_fixture() {
    assert_registry(&message_types(), &[("GOAWAY", "GoAway")], |cp| {
      ControlMessageType::try_from(cp)
        .ok()
        .map(|t| format!("{t:?}"))
    });
  }

  #[test]
  fn request_error_codes_match_fixture() {
    assert_registry(&request_error_codes(), &[], |cp| {
      RequestErrorCode::try_from(cp)
        .ok()
        .map(|c| format!("{c:?}"))
    });
  }

  #[test]
  fn setup_options_match_fixture() {
    assert_registry(&parameter_types().setup_options, &[], |cp| {
      SetupOptionType::try_from(cp).ok().map(|p| format!("{p:?}"))
    });
  }

  #[test]
  fn message_parameters_match_fixture() {
    assert_registry(&parameter_types().message_parameters, &[], |cp| {
      MessageParameterType::try_from(cp)
        .ok()
        .map(|p| format!("{p:?}"))
    });
  }

  #[test]
  fn property_types_match_fixture() {
    assert_registry(&property_types().registry, &[], |cp| {
      TrackPropertyType::try_from(cp)
        .ok()
        .map(|p| format!("{p:?}"))
    });
  }

  /// The LOC properties are registered in the same number space as the draft's own
  /// table (§15.8 Table 15), so they are held to it too.
  #[test]
  fn loc_property_ids_match_fixture() {
    assert_registry(&property_types().provisional, &[], |cp| {
      LOCPropertyId::try_from(cp).ok().map(|p| format!("{p:?}"))
    });
  }

  #[test]
  fn termination_codes_match_fixture() {
    // TerminationCode has no TryFrom<u64>, so the lookup is built from the variants.
    // This list carries no codepoints — the values still come only from the enum.
    const VARIANTS: &[TerminationCode] = &[
      TerminationCode::NoError,
      TerminationCode::InternalError,
      TerminationCode::Unauthorized,
      TerminationCode::ProtocolViolation,
      TerminationCode::InvalidRequestID,
      TerminationCode::DuplicateTrackAlias,
      TerminationCode::KeyValueFormattingError,
      TerminationCode::TooManyRequests,
      TerminationCode::InvalidPath,
      TerminationCode::MalformedPath,
      TerminationCode::GoawayTimeout,
      TerminationCode::ControlMessageTimeout,
      TerminationCode::DataStreamTimeout,
      TerminationCode::AuthTokenCacheOverflow,
      TerminationCode::DuplicateAuthTokenAlias,
      TerminationCode::VersionNegotiationFailed,
      TerminationCode::MalformedAuthToken,
      TerminationCode::UnknownAuthTokenAlias,
      TerminationCode::ExpiredAuthToken,
      TerminationCode::InvalidAuthority,
      TerminationCode::MalformedAuthority,
    ];

    assert_registry(
      &termination_codes(),
      &[("INVALID_REQUEST_ID", "InvalidRequestID")],
      |cp| {
        VARIANTS
          .iter()
          .find(|v| v.to_u32() as u64 == cp)
          .map(|v| format!("{v:?}"))
      },
    );
  }

  #[test]
  fn stream_reset_codes_match_fixture() {
    // This crate has no stream reset code enum yet, so every codepoint is unparsed.
    // The fixture marks all of them pending; this test fails the moment one lands
    // without its marker being cleared.
    assert_registry(&stream_reset_codes(), &[], |_| None);
  }

  #[test]
  fn pascal_case_matches_the_crate_convention() {
    assert_eq!(pascal("SUBSCRIBE_NAMESPACE"), "SubscribeNamespace");
    assert_eq!(pascal("PUBLISH_OK"), "PublishOk");
    assert_eq!(pascal("RESERVED_SETUP_V00"), "ReservedSetupV00");
    assert_eq!(pascal("SETUP"), "Setup");
  }
}
