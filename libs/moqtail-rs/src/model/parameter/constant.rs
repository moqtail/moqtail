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
pub enum SetupParameterType {
  Path = 0x01,
  MaxRequestId = 0x02,
  AuthorizationToken = 0x03,
  MaxAuthTokenCacheSize = 0x04,
  MoqtImplementation = 0x05,
}

impl TryFrom<u64> for SetupParameterType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x01 => Ok(SetupParameterType::Path),
      0x02 => Ok(SetupParameterType::MaxRequestId),
      0x03 => Ok(SetupParameterType::AuthorizationToken),
      0x04 => Ok(SetupParameterType::MaxAuthTokenCacheSize),
      0x05 => Ok(SetupParameterType::MoqtImplementation),
      _ => Err(ParseError::InvalidType {
        context: "SetupParameterType::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<SetupParameterType> for u64 {
  fn from(value: SetupParameterType) -> Self {
    value as u64
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum VersionSpecificParameterType {
  DeliveryTimeout = 0x02,
  AuthorizationToken = 0x03,
  MaxCacheDuration = 0x04,
  ForwardActionGroup = 0x40,
}

impl TryFrom<u64> for VersionSpecificParameterType {
  type Error = ParseError;

  fn try_from(value: u64) -> Result<Self, Self::Error> {
    match value {
      0x02 => Ok(VersionSpecificParameterType::DeliveryTimeout),
      0x03 => Ok(VersionSpecificParameterType::AuthorizationToken),
      0x04 => Ok(VersionSpecificParameterType::MaxCacheDuration),
      0x40 => Ok(VersionSpecificParameterType::ForwardActionGroup),
      _ => Err(ParseError::InvalidType {
        context: "VersionSpecificParameterType::try_from(u64)",
        details: format!("Invalid type, got {value}"),
      }),
    }
  }
}

impl From<VersionSpecificParameterType> for u64 {
  fn from(value: VersionSpecificParameterType) -> Self {
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
