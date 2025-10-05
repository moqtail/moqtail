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

pub const DRAFT_11: u32 = 0xFF00000B;
pub const DRAFT_14: u32 = 0xFF00000E;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ControlMessageType {
  ClientSetup = 0x20,
  ServerSetup = 0x21,
  GoAway = 0x10,
  MaxRequestId = 0x15,
  RequestsBlocked = 0x1A,
  Subscribe = 0x03,
  SubscribeOk = 0x04,
  SubscribeError = 0x05,
  Unsubscribe = 0x0A,
  SubscribeUpdate = 0x02,
  Fetch = 0x16,
  FetchOk = 0x18,
  FetchError = 0x19,
  FetchCancel = 0x17,
  TrackStatus = 0x0D,
  TrackStatusOk = 0x0E,
  TrackStatusError = 0x0F,
  PublishNamespace = 0x06,
  PublishNamespaceOk = 0x07,
  PublishNamespaceError = 0x08,
  PublishNamespaceDone = 0x09,
  PublishNamespaceCancel = 0x0C,
  SubscribeNamespace = 0x11,
  SubscribeNamespaceOk = 0x12,
  SubscribeNamespaceError = 0x13,
  UnsubscribeNamespace = 0x14,
  Publish = 0x1D,
  PublishDone = 0x0B,
  PublishOk = 0x1E,
  PublishError = 0x1F,
}

impl TryFrom<u64> for ControlMessageType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x20 => Ok(ControlMessageType::ClientSetup),
      0x21 => Ok(ControlMessageType::ServerSetup),
      0x10 => Ok(ControlMessageType::GoAway),
      0x15 => Ok(ControlMessageType::MaxRequestId),
      0x1A => Ok(ControlMessageType::RequestsBlocked),
      0x03 => Ok(ControlMessageType::Subscribe),
      0x04 => Ok(ControlMessageType::SubscribeOk),
      0x05 => Ok(ControlMessageType::SubscribeError),
      0x0A => Ok(ControlMessageType::Unsubscribe),
      0x02 => Ok(ControlMessageType::SubscribeUpdate),
      0x0B => Ok(ControlMessageType::PublishDone),
      0x16 => Ok(ControlMessageType::Fetch),
      0x18 => Ok(ControlMessageType::FetchOk),
      0x19 => Ok(ControlMessageType::FetchError),
      0x17 => Ok(ControlMessageType::FetchCancel),
      0x0D => Ok(ControlMessageType::TrackStatus),
      0x0E => Ok(ControlMessageType::TrackStatus),
      0x06 => Ok(ControlMessageType::PublishNamespace),
      0x07 => Ok(ControlMessageType::PublishNamespaceOk),
      0x08 => Ok(ControlMessageType::PublishNamespaceError),
      0x09 => Ok(ControlMessageType::PublishNamespaceDone),
      0x0C => Ok(ControlMessageType::PublishNamespaceCancel),
      0x11 => Ok(ControlMessageType::SubscribeNamespace),
      0x12 => Ok(ControlMessageType::SubscribeNamespaceOk),
      0x13 => Ok(ControlMessageType::SubscribeNamespaceError),
      0x14 => Ok(ControlMessageType::UnsubscribeNamespace),
      0x1D => Ok(ControlMessageType::Publish),
      0x1E => Ok(ControlMessageType::PublishOk),
      0x1F => Ok(ControlMessageType::PublishError),
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
  StandAlone = 0x1,
  RelativeFetch = 0x2,
  AbsoluteFetch = 0x3,
}

impl TryFrom<u64> for FetchType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x1 => Ok(FetchType::StandAlone),
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
pub enum SubscribeErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  TrackDoesNotExist = 0x4,
  InvalidRange = 0x5,
  MalformedAuthToken = 0x10,
  ExpiredAuthToken = 0x12,
}

impl TryFrom<u64> for SubscribeErrorCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(SubscribeErrorCode::InternalError),
      0x1 => Ok(SubscribeErrorCode::Unauthorized),
      0x2 => Ok(SubscribeErrorCode::Timeout),
      0x3 => Ok(SubscribeErrorCode::NotSupported),
      0x4 => Ok(SubscribeErrorCode::TrackDoesNotExist),
      0x5 => Ok(SubscribeErrorCode::InvalidRange),
      0x10 => Ok(SubscribeErrorCode::MalformedAuthToken),
      0x12 => Ok(SubscribeErrorCode::ExpiredAuthToken),
      _ => Err(ParseError::InvalidType {
        context: "SubscribeErrorCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<SubscribeErrorCode> for u64 {
  fn from(value: SubscribeErrorCode) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum FetchErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  TrackDoesNotExist = 0x4,
  InvalidRange = 0x5,
  NoObjects = 0x6,
  InvalidJoiningRequestId = 0x7,
  UnknownStatusInRange = 0x8,
  MalformedTrack = 0x9,
  MalformedAuthToken = 0x10,
  ExpiredAuthToken = 0x12,
}

impl TryFrom<u64> for FetchErrorCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(FetchErrorCode::InternalError),
      0x1 => Ok(FetchErrorCode::Unauthorized),
      0x2 => Ok(FetchErrorCode::Timeout),
      0x3 => Ok(FetchErrorCode::NotSupported),
      0x4 => Ok(FetchErrorCode::TrackDoesNotExist),
      0x5 => Ok(FetchErrorCode::InvalidRange),
      0x6 => Ok(FetchErrorCode::NoObjects),
      0x7 => Ok(FetchErrorCode::InvalidJoiningRequestId),
      0x8 => Ok(FetchErrorCode::UnknownStatusInRange),
      0x9 => Ok(FetchErrorCode::MalformedTrack),
      0x10 => Ok(FetchErrorCode::MalformedAuthToken),
      0x12 => Ok(FetchErrorCode::ExpiredAuthToken),
      _ => Err(ParseError::InvalidType {
        context: "FetchErrorCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<FetchErrorCode> for u64 {
  fn from(value: FetchErrorCode) -> Self {
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
pub enum SubscribeNamespaceErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  NamespacePrefixUnknown = 0x4,
  NamespacePrefixOverlap = 0x5,
  MalformedAuthToken = 0x10,
  UnknownAuthTokenAlias = 0x11,
  ExpiredAuthToken = 0x12,
}
impl TryFrom<u64> for SubscribeNamespaceErrorCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(SubscribeNamespaceErrorCode::InternalError),
      0x1 => Ok(SubscribeNamespaceErrorCode::Unauthorized),
      0x2 => Ok(SubscribeNamespaceErrorCode::Timeout),
      0x3 => Ok(SubscribeNamespaceErrorCode::NotSupported),
      0x4 => Ok(SubscribeNamespaceErrorCode::NamespacePrefixUnknown),
      0x5 => Ok(SubscribeNamespaceErrorCode::NamespacePrefixOverlap),
      0x10 => Ok(SubscribeNamespaceErrorCode::MalformedAuthToken),
      0x11 => Ok(SubscribeNamespaceErrorCode::UnknownAuthTokenAlias),
      0x12 => Ok(SubscribeNamespaceErrorCode::ExpiredAuthToken),
      _ => Err(ParseError::InvalidType {
        context: "SubscribeNamespaceErrorCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<SubscribeNamespaceErrorCode> for u64 {
  fn from(value: SubscribeNamespaceErrorCode) -> Self {
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
pub enum PublishNamespaceErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  Uninterested = 0x4,
  MalformedAuthToken = 0x10,
  UnknownAuthTokenAlias = 0x11,
  ExpiredAuthToken = 0x12,
}

impl TryFrom<u64> for PublishNamespaceErrorCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(PublishNamespaceErrorCode::InternalError),
      0x1 => Ok(PublishNamespaceErrorCode::Unauthorized),
      0x2 => Ok(PublishNamespaceErrorCode::Timeout),
      0x3 => Ok(PublishNamespaceErrorCode::NotSupported),
      0x4 => Ok(PublishNamespaceErrorCode::Uninterested),
      0x10 => Ok(PublishNamespaceErrorCode::MalformedAuthToken),
      0x11 => Ok(PublishNamespaceErrorCode::UnknownAuthTokenAlias),
      0x12 => Ok(PublishNamespaceErrorCode::ExpiredAuthToken),
      _ => Err(ParseError::InvalidType {
        context: "PublishNamespaceErrorCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<PublishNamespaceErrorCode> for u64 {
  fn from(value: PublishNamespaceErrorCode) -> Self {
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
  MalformedTrack = 0x7,
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
      0x7 => Ok(PublishDoneStatusCode::MalformedTrack),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum PublishErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  Uninterested = 0x4,
}

impl TryFrom<u64> for PublishErrorCode {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(PublishErrorCode::InternalError),
      0x1 => Ok(PublishErrorCode::Unauthorized),
      0x2 => Ok(PublishErrorCode::Timeout),
      0x3 => Ok(PublishErrorCode::NotSupported),
      0x4 => Ok(PublishErrorCode::Uninterested),
      _ => Err(ParseError::InvalidType {
        context: "PublishErrorCode::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<PublishErrorCode> for u64 {
  fn from(value: PublishErrorCode) -> Self {
    value as u64
  }
}
