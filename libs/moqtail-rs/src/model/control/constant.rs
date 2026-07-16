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

use core::convert::From;

use crate::model::error::ParseError;

pub const SUPPORTED_VERSIONS: &str = "moqt-18";

/// Control message types, per draft-18 Table 5.
///
/// The comment on each variant is the Stream column: `Control` is the control stream
/// (§3.3), `Request` a bidirectional request stream, and `First` means the message MUST
/// be the first on a new request stream.
///
/// Table 5 also reserves `0x01` (SETUP for version 00), `0x40`/`0x41` (CLIENT_SETUP /
/// SERVER_SETUP for version <= 10) and `0x20`/`0x21` (CLIENT_SETUP / SERVER_SETUP in
/// version <= 16). Reserved codepoints are deliberately absent from this enum:
/// `TryFrom` rejects them, which is what §10 requires — an endpoint receiving an
/// unknown message type MUST close the session.
///
/// The values here are asserted against `dev/conformance/draft18/message_types.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ControlMessageType {
  Setup = 0x2F00,                // Control
  GoAway = 0x10,                 // Control, Request
  MaxRequestId = 0x15,           // not in draft-18
  RequestsBlocked = 0x1A,        // not in draft-18
  Subscribe = 0x03,              // Request, First
  SubscribeOk = 0x04,            // Request
  RequestError = 0x05,           // Request
  Unsubscribe = 0x0A,            // not in draft-18
  RequestUpdate = 0x02,          // Request
  Fetch = 0x16,                  // Request, First
  FetchOk = 0x18,                // Request
  FetchCancel = 0x17,            // not in draft-18
  TrackStatus = 0x0D,            // Request, First
  PublishNamespace = 0x06,       // Request, First
  RequestOk = 0x07,              // Request
  Namespace = 0x08,              // Request
  PublishNamespaceDone = 0x09,   // not in draft-18
  NamespaceDone = 0x0E,          // Request
  PublishNamespaceCancel = 0x0C, // not in draft-18
  SubscribeNamespace = 0x50,     // Request, First
  SubscribeTracks = 0x51,        // Request, First
  UnsubscribeNamespace = 0x14,   // not in draft-18
  Publish = 0x1D,                // Request, First
  PublishDone = 0x0B,            // Request
  PublishOk = 0x1E,              // Request; an alias of RequestOk (§10.5), not its own body
  PublishBlocked = 0x0F,         // Request
  Switch = 0x22,                 // not in draft-18; moqtail-local extension
}

impl TryFrom<u64> for ControlMessageType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x2F00 => Ok(ControlMessageType::Setup),
      0x10 => Ok(ControlMessageType::GoAway),
      0x15 => Ok(ControlMessageType::MaxRequestId),
      0x1A => Ok(ControlMessageType::RequestsBlocked),
      0x03 => Ok(ControlMessageType::Subscribe),
      0x04 => Ok(ControlMessageType::SubscribeOk),
      0x05 => Ok(ControlMessageType::RequestError),
      0x0A => Ok(ControlMessageType::Unsubscribe),
      0x02 => Ok(ControlMessageType::RequestUpdate),
      0x0B => Ok(ControlMessageType::PublishDone),
      0x16 => Ok(ControlMessageType::Fetch),
      0x18 => Ok(ControlMessageType::FetchOk),
      0x17 => Ok(ControlMessageType::FetchCancel),
      0x0D => Ok(ControlMessageType::TrackStatus),
      0x06 => Ok(ControlMessageType::PublishNamespace),
      0x07 => Ok(ControlMessageType::RequestOk),
      0x08 => Ok(ControlMessageType::Namespace),
      0x09 => Ok(ControlMessageType::PublishNamespaceDone),
      0x0E => Ok(ControlMessageType::NamespaceDone),
      0x0C => Ok(ControlMessageType::PublishNamespaceCancel),
      0x50 => Ok(ControlMessageType::SubscribeNamespace),
      0x51 => Ok(ControlMessageType::SubscribeTracks),
      0x14 => Ok(ControlMessageType::UnsubscribeNamespace),
      0x1D => Ok(ControlMessageType::Publish),
      0x1E => Ok(ControlMessageType::PublishOk),
      0x0F => Ok(ControlMessageType::PublishBlocked),
      0x22 => Ok(ControlMessageType::Switch),
      _ => Err(ParseError::InvalidType {
        context: " ControlMessageType::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<ControlMessageType> for u64 {
  fn from(value: ControlMessageType) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u64)]
pub enum FilterType {
  NextGroupStart = 0x1,
  LatestObject = 0x2,
  AbsoluteStart = 0x3,
  AbsoluteRange = 0x4,
}

impl TryFrom<u64> for FilterType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x1 => Ok(FilterType::NextGroupStart),
      0x2 => Ok(FilterType::LatestObject),
      0x3 => Ok(FilterType::AbsoluteStart),
      0x4 => Ok(FilterType::AbsoluteRange),
      _ => Err(ParseError::InvalidType {
        context: "FilterType::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<FilterType> for u64 {
  fn from(value: FilterType) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u64)]
pub enum FetchType {
  Standalone = 0x1,
  RelativeFetch = 0x2,
  AbsoluteFetch = 0x3,
}

impl TryFrom<u64> for FetchType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x1 => Ok(FetchType::Standalone),
      0x2 => Ok(FetchType::RelativeFetch),
      0x3 => Ok(FetchType::AbsoluteFetch),
      _ => Err(ParseError::InvalidType {
        context: "FetchType::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<FetchType> for u64 {
  fn from(value: FetchType) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum GroupOrder {
  Original = 0x0,
  Ascending = 0x1,
  Descending = 0x2,
}

impl TryFrom<u8> for GroupOrder {
  type Error = ParseError;

  fn try_from(value: u8) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(GroupOrder::Original),
      0x1 => Ok(GroupOrder::Ascending),
      0x2 => Ok(GroupOrder::Descending),
      _ => Err(ParseError::InvalidType {
        context: "GroupOrder::try_from(u8)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<GroupOrder> for u8 {
  fn from(value: GroupOrder) -> Self {
    value as u8
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum RequestErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  MalformedAuthToken = 0x4,
  ExpiredAuthToken = 0x5,
  DoesNotExist = 0x10,
  InvalidRange = 0x11,
  MalformedTrack = 0x12,
  DuplicateSubscription = 0x19,
  Uninterested = 0x20,
  PrefixOverlap = 0x30,
  InvalidJoiningRequestId = 0x32,
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
      0x10 => Ok(RequestErrorCode::DoesNotExist),
      0x11 => Ok(RequestErrorCode::InvalidRange),
      0x12 => Ok(RequestErrorCode::MalformedTrack),
      0x19 => Ok(RequestErrorCode::DuplicateSubscription),
      0x20 => Ok(RequestErrorCode::Uninterested),
      0x30 => Ok(RequestErrorCode::PrefixOverlap),
      0x32 => Ok(RequestErrorCode::InvalidJoiningRequestId),
      _ => Err(ParseError::InvalidType {
        context: "RequestErrorCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<RequestErrorCode> for u64 {
  fn from(value: RequestErrorCode) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum TrackStatusCode {
  InProgress = 0x00,
  DoesNotExist = 0x01,
  NotYetBegun = 0x02,
  Finished = 0x03,
  RelayUnavailable = 0x04,
}

impl TryFrom<u64> for TrackStatusCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x00 => Ok(TrackStatusCode::InProgress),
      0x01 => Ok(TrackStatusCode::DoesNotExist),
      0x02 => Ok(TrackStatusCode::NotYetBegun),
      0x03 => Ok(TrackStatusCode::Finished),
      0x04 => Ok(TrackStatusCode::RelayUnavailable),
      _ => Err(ParseError::InvalidType {
        context: "TrackStatusCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<TrackStatusCode> for u64 {
  fn from(value: TrackStatusCode) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum SubscribeDoneStatusCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  TrackEnded = 0x2,
  SubscriptionEnded = 0x3,
  GoingAway = 0x4,
  Expired = 0x5,
  TooFarBehind = 0x6,
}
impl TryFrom<u64> for SubscribeDoneStatusCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(SubscribeDoneStatusCode::InternalError),
      0x1 => Ok(SubscribeDoneStatusCode::Unauthorized),
      0x2 => Ok(SubscribeDoneStatusCode::TrackEnded),
      0x3 => Ok(SubscribeDoneStatusCode::SubscriptionEnded),
      0x4 => Ok(SubscribeDoneStatusCode::GoingAway),
      0x5 => Ok(SubscribeDoneStatusCode::Expired),
      0x6 => Ok(SubscribeDoneStatusCode::TooFarBehind),
      _ => Err(ParseError::InvalidType {
        context: "SubscribeDoneStatusCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<SubscribeDoneStatusCode> for u64 {
  fn from(value: SubscribeDoneStatusCode) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum PublishDoneStatusCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  TrackEnded = 0x2,
  SubscriptionEnded = 0x3,
  GoingAway = 0x4,
  Expired = 0x5,
  TooFarBehind = 0x6,
  UpdateFailed = 0x8,
  MalformedTrack = 0x12,
}

impl TryFrom<u64> for PublishDoneStatusCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(PublishDoneStatusCode::InternalError),
      0x1 => Ok(PublishDoneStatusCode::Unauthorized),
      0x2 => Ok(PublishDoneStatusCode::TrackEnded),
      0x3 => Ok(PublishDoneStatusCode::SubscriptionEnded),
      0x4 => Ok(PublishDoneStatusCode::GoingAway),
      0x5 => Ok(PublishDoneStatusCode::Expired),
      0x6 => Ok(PublishDoneStatusCode::TooFarBehind),
      0x8 => Ok(PublishDoneStatusCode::UpdateFailed),
      0x12 => Ok(PublishDoneStatusCode::MalformedTrack),
      _ => Err(ParseError::InvalidType {
        context: "PublishDoneStatusCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<PublishDoneStatusCode> for u64 {
  fn from(value: PublishDoneStatusCode) -> Self {
    value as u64
  }
}
