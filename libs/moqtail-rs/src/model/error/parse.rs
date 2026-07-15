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

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
  #[error("[{context}] not enough bytes: needed {needed}, available {available}")]
  NotEnoughBytes {
    context: &'static str,
    needed: usize,
    available: usize,
  },

  #[error("[{context}] cannot cast from {from_type} to {to_type}, [{details}]")]
  CastingError {
    context: &'static str,
    from_type: &'static str,
    to_type: &'static str,
    details: String,
  },

  #[error("[{context}] length {len} exceeds maximum of {max}, protocol violation")]
  LengthExceedsMax {
    context: &'static str,
    max: usize,
    len: usize,
  },

  #[error("[{context}] key value formatting error")]
  KeyValueFormattingError { context: &'static str },

  #[error("Invalid type: [{context}], [{details}]")]
  InvalidType {
    context: &'static str,
    details: String,
  },

  #[error("Invalid UTF8: [{context}], [{details}]")]
  InvalidUTF8 {
    context: &'static str,
    details: String,
  },

  #[error("Protocol violation: [{context}], [{details}]")]
  ProtocolViolation {
    context: &'static str,
    details: String,
  },
  #[error("Track naming error: [{context}], [{details}]")]
  TrackNameError {
    context: &'static str,
    details: String,
  },
  #[error("Track naming error: [{context}], [{details}]")]
  TrackAliasError {
    context: &'static str,
    details: String,
  },
  #[error("Timeout: [{context}]")]
  Timeout { context: &'static str },
  #[error("[{context}], [{msg}]")]
  Other { context: &'static str, msg: String },
}
