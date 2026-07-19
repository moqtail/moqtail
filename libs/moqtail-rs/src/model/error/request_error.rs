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

//! Error codes carried by REQUEST_ERROR (§10.6).
//!
//! This is a distinct registry from stream reset codes; the two disagree on some
//! names — e.g. `GOING_AWAY` is `0x6` here but `0x4` as a stream reset code.

use crate::model::error::ParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum RequestErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  MalformedAuthToken = 0x4,
  ExpiredAuthToken = 0x5,
  GoingAway = 0x6,
  ExcessiveLoad = 0x9,
  DoesNotExist = 0x10,
  InvalidRange = 0x11,
  MalformedTrack = 0x12,
  DuplicateSubscription = 0x19,
  Uninterested = 0x20,
  PrefixOverlap = 0x30,
  NamespaceTooLarge = 0x31,
  InvalidJoiningRequestId = 0x32,
  UnsupportedExtension = 0x33,
}

impl TryFrom<u64> for RequestErrorCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(RequestErrorCode::InternalError),
      0x1 => Ok(RequestErrorCode::Unauthorized),
      0x2 => Ok(RequestErrorCode::Timeout),
      0x3 => Ok(RequestErrorCode::NotSupported),
      0x4 => Ok(RequestErrorCode::MalformedAuthToken),
      0x5 => Ok(RequestErrorCode::ExpiredAuthToken),
      0x6 => Ok(RequestErrorCode::GoingAway),
      0x9 => Ok(RequestErrorCode::ExcessiveLoad),
      0x10 => Ok(RequestErrorCode::DoesNotExist),
      0x11 => Ok(RequestErrorCode::InvalidRange),
      0x12 => Ok(RequestErrorCode::MalformedTrack),
      0x19 => Ok(RequestErrorCode::DuplicateSubscription),
      0x20 => Ok(RequestErrorCode::Uninterested),
      0x30 => Ok(RequestErrorCode::PrefixOverlap),
      0x31 => Ok(RequestErrorCode::NamespaceTooLarge),
      0x32 => Ok(RequestErrorCode::InvalidJoiningRequestId),
      0x33 => Ok(RequestErrorCode::UnsupportedExtension),
      _ => Err(ParseError::InvalidType {
        context: "RequestErrorCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl RequestErrorCode {
  /// Maps a received error code to a known variant, treating any unknown value
  /// (including GREASE) as `InternalError`. An unknown error code is never fatal
  /// and never closes the session.
  pub fn from_wire(value: u64) -> Self {
    Self::try_from(value).unwrap_or(Self::InternalError)
  }
}

impl From<RequestErrorCode> for u64 {
  fn from(value: RequestErrorCode) -> Self {
    value as u64
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::grease::grease_value;

  #[test]
  fn from_wire_maps_unknown_and_grease_to_internal_error() {
    assert_eq!(
      RequestErrorCode::from_wire(0x1),
      RequestErrorCode::Unauthorized
    );
    for raw in [0x7Eu64, grease_value(0).unwrap(), grease_value(5).unwrap()] {
      assert_eq!(
        RequestErrorCode::from_wire(raw),
        RequestErrorCode::InternalError
      );
    }
  }
}
