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

//! Error codes carried when resetting a request stream (or sending STOP_SENDING).
//!
//! This is a distinct registry from REQUEST_ERROR codes and they deliberately
//! disagree on the same names — e.g. `GOING_AWAY` is `0x4` here but `0x6` as a
//! REQUEST_ERROR code. Do not merge the two.

use crate::model::error::ParseError;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[repr(u64)]
pub enum StreamResetCode {
  InternalError = 0x0,
  Cancelled = 0x1,
  DeliveryTimeout = 0x2,
  SessionClosed = 0x3,
  GoingAway = 0x4,
  TooFarBehind = 0x5,
  UnknownObjectStatus = 0x6,
  ExpiredAuthToken = 0x7,
  ExcessiveLoad = 0x9,
  MalformedTrack = 0x12,
}

impl StreamResetCode {
  /// The numeric code carried on the wire (QUIC application error code).
  pub fn to_u64(self) -> u64 {
    self as u64
  }
}

impl TryFrom<u64> for StreamResetCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(StreamResetCode::InternalError),
      0x1 => Ok(StreamResetCode::Cancelled),
      0x2 => Ok(StreamResetCode::DeliveryTimeout),
      0x3 => Ok(StreamResetCode::SessionClosed),
      0x4 => Ok(StreamResetCode::GoingAway),
      0x5 => Ok(StreamResetCode::TooFarBehind),
      0x6 => Ok(StreamResetCode::UnknownObjectStatus),
      0x7 => Ok(StreamResetCode::ExpiredAuthToken),
      0x9 => Ok(StreamResetCode::ExcessiveLoad),
      0x12 => Ok(StreamResetCode::MalformedTrack),
      _ => Err(ParseError::InvalidType {
        context: "StreamResetCode::try_from(u64)",
        details: format!("Invalid stream reset code, got {value}"),
      }),
    }
  }
}

impl From<StreamResetCode> for u64 {
  fn from(value: StreamResetCode) -> Self {
    value as u64
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn roundtrips_every_code() {
    for code in [
      StreamResetCode::InternalError,
      StreamResetCode::Cancelled,
      StreamResetCode::DeliveryTimeout,
      StreamResetCode::SessionClosed,
      StreamResetCode::GoingAway,
      StreamResetCode::TooFarBehind,
      StreamResetCode::UnknownObjectStatus,
      StreamResetCode::ExpiredAuthToken,
      StreamResetCode::ExcessiveLoad,
      StreamResetCode::MalformedTrack,
    ] {
      assert_eq!(StreamResetCode::try_from(code.to_u64()).unwrap(), code);
    }
    assert!(StreamResetCode::try_from(0x8).is_err());
  }
}
