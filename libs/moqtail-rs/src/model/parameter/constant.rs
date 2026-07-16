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

use crate::model::error::ParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum SetupOptionType {
  Path = 0x01,
  MaxRequestId = 0x02,
  AuthorizationToken = 0x03,
  MaxAuthTokenCacheSize = 0x04,
  Authority = 0x05, // MQOtail does not use this (WebTransport)
  MoqtImplementation = 0x07,
}

impl TryFrom<u64> for SetupOptionType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x01 => Ok(SetupOptionType::Path),
      0x02 => Ok(SetupOptionType::MaxRequestId),
      0x03 => Ok(SetupOptionType::AuthorizationToken),
      0x04 => Ok(SetupOptionType::MaxAuthTokenCacheSize),
      0x05 => Ok(SetupOptionType::Authority),
      0x07 => Ok(SetupOptionType::MoqtImplementation),
      _ => Err(ParseError::InvalidType {
        context: "SetupOptionType::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<SetupOptionType> for u64 {
  fn from(value: SetupOptionType) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum MessageParameterType {
  ObjectDeliveryTimeout = 0x02,
  AuthorizationToken = 0x03,
  RendezvousTimeout = 0x04,
  SubgroupDeliveryTimeout = 0x06,
  Expires = 0x08,
  LargestObject = 0x09,
  FillTimeout = 0x0A,
  Forward = 0x10,
  SubscriberPriority = 0x20,
  SubscriptionFilter = 0x21,
  GroupOrder = 0x22,
  NewGroupRequest = 0x32,
  TrackNamespacePrefix = 0x34,
}

impl TryFrom<u64> for MessageParameterType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x02 => Ok(MessageParameterType::ObjectDeliveryTimeout),
      0x03 => Ok(MessageParameterType::AuthorizationToken),
      0x04 => Ok(MessageParameterType::RendezvousTimeout),
      0x06 => Ok(MessageParameterType::SubgroupDeliveryTimeout),
      0x08 => Ok(MessageParameterType::Expires),
      0x09 => Ok(MessageParameterType::LargestObject),
      0x0A => Ok(MessageParameterType::FillTimeout),
      0x10 => Ok(MessageParameterType::Forward),
      0x20 => Ok(MessageParameterType::SubscriberPriority),
      0x21 => Ok(MessageParameterType::SubscriptionFilter),
      0x22 => Ok(MessageParameterType::GroupOrder),
      0x32 => Ok(MessageParameterType::NewGroupRequest),
      0x34 => Ok(MessageParameterType::TrackNamespacePrefix),
      _ => Err(ParseError::InvalidType {
        context: "MessageParameterType::try_from(u64)",
        details: format!("Unknown parameter type, got {value}"),
      }),
    }
  }
}

impl From<MessageParameterType> for u64 {
  fn from(value: MessageParameterType) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum TokenAliasType {
  Delete = 0x0,
  Register = 0x1,
  UseAlias = 0x2,
  UseValue = 0x3,
}
impl TryFrom<u64> for TokenAliasType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(TokenAliasType::Delete),
      0x1 => Ok(TokenAliasType::Register),
      0x2 => Ok(TokenAliasType::UseAlias),
      0x3 => Ok(TokenAliasType::UseValue),
      _ => Err(ParseError::InvalidType {
        context: "TokenAliasType::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<TokenAliasType> for u64 {
  fn from(value: TokenAliasType) -> Self {
    value as u64
  }
}
