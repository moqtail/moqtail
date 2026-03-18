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

use crate::model::common::location::Location;
use crate::model::common::pair::KeyValuePair;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::control::constant::{FilterType, GroupOrder};
use crate::model::error::ParseError;
use bytes::{Buf, Bytes, BytesMut};

// --- Draft-16 Magic Numbers ---
pub const PARAM_DELIVERY_TIMEOUT: u64 = 0x02;
pub const PARAM_AUTHORIZATION_TOKEN: u64 = 0x03;
pub const PARAM_EXPIRES: u64 = 0x08;
pub const PARAM_LARGEST_OBJECT: u64 = 0x09;
pub const PARAM_FORWARD: u64 = 0x10;
pub const PARAM_SUBSCRIBER_PRIORITY: u64 = 0x20;
pub const PARAM_SUBSCRIPTION_FILTER: u64 = 0x21;
pub const PARAM_GROUP_ORDER: u64 = 0x22;
pub const PARAM_NEW_GROUP_REQUEST: u64 = 0x32;

#[derive(Debug, Clone, PartialEq)]
pub struct MessageParameters {
  pub delivery_timeout: Option<u64>,
  pub authorization_token: Option<Bytes>,
  pub expires: Option<u64>,
  pub largest_object: Option<Location>,

  pub forward: bool,
  pub subscriber_priority: u8,
  pub group_order: GroupOrder,

  pub filter_type: FilterType,
  pub start_location: Option<Location>,
  pub end_group: Option<u64>,

  pub new_group_request: Option<u64>,

  pub unknown_parameters: Vec<KeyValuePair>,
}

impl Default for MessageParameters {
  fn default() -> Self {
    Self {
      delivery_timeout: None,
      authorization_token: None,
      expires: None,
      largest_object: None,
      forward: true,            // Draft 9.2.2.8 Default
      subscriber_priority: 128, // Draft 9.2.2.3 Default
      group_order: GroupOrder::Original,
      filter_type: FilterType::LatestObject,
      start_location: None,
      end_group: None,
      new_group_request: None,
      unknown_parameters: Vec::new(),
    }
  }
}

impl MessageParameters {
  pub fn new() -> Self {
    Self::default()
  }

  /// Parses directly from a byte buffer, skipping intermediate vector allocations for known fields.
  pub fn deserialize(bytes: &mut Bytes, param_count: u64) -> Result<Self, ParseError> {
    let mut params = Self::default();

    for _ in 0..param_count {
      let param_type = bytes.get_vi()?;

      if param_type % 2 == 0 {
        let value = bytes.get_vi()?;
        match param_type {
          PARAM_FORWARD => params.forward = value == 1,
          PARAM_SUBSCRIBER_PRIORITY => params.subscriber_priority = value as u8,
          PARAM_GROUP_ORDER => {
            params.group_order = GroupOrder::try_from(value as u8).unwrap_or(GroupOrder::Original)
          }
          PARAM_DELIVERY_TIMEOUT => params.delivery_timeout = Some(value),
          PARAM_EXPIRES => params.expires = Some(value),
          PARAM_NEW_GROUP_REQUEST => params.new_group_request = Some(value),
          _ => params
            .unknown_parameters
            .push(KeyValuePair::try_new_varint(param_type, value)?),
        }
      } else {
        let len = bytes.get_vi()? as usize;
        if bytes.remaining() < len {
          return Err(ParseError::NotEnoughBytes {
            context: "MessageParameters::deserialize",
            needed: len,
            available: bytes.remaining(),
          });
        }
        let mut value_bytes = bytes.copy_to_bytes(len);

        match param_type {
          PARAM_AUTHORIZATION_TOKEN => params.authorization_token = Some(value_bytes),
          PARAM_LARGEST_OBJECT => {
            if let Ok(loc) = Location::deserialize(&mut value_bytes) {
              params.largest_object = Some(loc);
            }
          }
          PARAM_SUBSCRIPTION_FILTER => {
            if let Ok(ft_raw) = value_bytes.get_vi()
              && let Ok(ft) = FilterType::try_from(ft_raw)
            {
              params.filter_type = ft;
              match ft {
                FilterType::AbsoluteStart => {
                  if let Ok(loc) = Location::deserialize(&mut value_bytes) {
                    params.start_location = Some(loc);
                  }
                }
                FilterType::AbsoluteRange => {
                  if let Ok(loc) = Location::deserialize(&mut value_bytes) {
                    params.start_location = Some(loc);
                    if let Ok(eg) = value_bytes.get_vi() {
                      params.end_group = Some(eg);
                    }
                  }
                }
                _ => {}
              }
            }
          }
          _ => params
            .unknown_parameters
            .push(KeyValuePair::try_new_bytes(param_type, value_bytes)?),
        }
      }
    }
    Ok(params)
  }

  pub fn into_vec(self) -> Result<Vec<KeyValuePair>, ParseError> {
    let mut pairs = self.unknown_parameters;

    if !self.forward {
      pairs.push(KeyValuePair::try_new_varint(PARAM_FORWARD, 0)?);
    }
    if self.subscriber_priority != 128 {
      pairs.push(KeyValuePair::try_new_varint(
        PARAM_SUBSCRIBER_PRIORITY,
        self.subscriber_priority as u64,
      )?);
    }
    if self.group_order != GroupOrder::Original {
      pairs.push(KeyValuePair::try_new_varint(
        PARAM_GROUP_ORDER,
        self.group_order as u64,
      )?);
    }

    // Serialize Options
    if let Some(to) = self.delivery_timeout {
      pairs.push(KeyValuePair::try_new_varint(PARAM_DELIVERY_TIMEOUT, to)?);
    }
    if let Some(exp) = self.expires {
      pairs.push(KeyValuePair::try_new_varint(PARAM_EXPIRES, exp)?);
    }
    if let Some(ngr) = self.new_group_request {
      pairs.push(KeyValuePair::try_new_varint(PARAM_NEW_GROUP_REQUEST, ngr)?);
    }
    if let Some(token) = self.authorization_token {
      pairs.push(KeyValuePair::try_new_bytes(
        PARAM_AUTHORIZATION_TOKEN,
        token,
      )?);
    }

    if let Some(loc) = self.largest_object {
      let mut buf = BytesMut::new();
      buf.put_vi(loc.group)?;
      buf.put_vi(loc.object)?;
      pairs.push(KeyValuePair::try_new_bytes(
        PARAM_LARGEST_OBJECT,
        buf.freeze(),
      )?);
    }

    if self.filter_type != FilterType::LatestObject {
      let mut buf = BytesMut::new();
      buf.put_vi(self.filter_type as u64)?;

      if (self.filter_type == FilterType::AbsoluteStart
        || self.filter_type == FilterType::AbsoluteRange)
        && let Some(loc) = self.start_location
      {
        buf.put_vi(loc.group)?;
        buf.put_vi(loc.object)?;
      }
      if self.filter_type == FilterType::AbsoluteRange
        && let Some(eg) = self.end_group
      {
        buf.put_vi(eg)?;
      }
      pairs.push(KeyValuePair::try_new_bytes(
        PARAM_SUBSCRIPTION_FILTER,
        buf.freeze(),
      )?);
    }

    Ok(pairs)
  }

  pub fn apply_update(&mut self, updates: Vec<KeyValuePair>) {
    for param in updates {
      match param.get_type() {
        PARAM_FORWARD => {
          if let KeyValuePair::VarInt { value, .. } = param {
            self.forward = value == 1;
          }
        }
        PARAM_SUBSCRIBER_PRIORITY => {
          if let KeyValuePair::VarInt { value, .. } = param {
            self.subscriber_priority = value as u8;
          }
        }
        PARAM_GROUP_ORDER => {
          if let KeyValuePair::VarInt { value, .. } = param {
            self.group_order = GroupOrder::try_from(value as u8).unwrap_or(GroupOrder::Original);
          }
        }
        PARAM_DELIVERY_TIMEOUT => {
          if let KeyValuePair::VarInt { value, .. } = param {
            self.delivery_timeout = Some(value);
          }
        }
        PARAM_EXPIRES => {
          if let KeyValuePair::VarInt { value, .. } = param {
            self.expires = Some(value);
          }
        }
        PARAM_NEW_GROUP_REQUEST => {
          if let KeyValuePair::VarInt { value, .. } = param {
            self.new_group_request = Some(value);
          }
        }
        PARAM_LARGEST_OBJECT => {
          if let KeyValuePair::Bytes { mut value, .. } = param
            && let Ok(loc) = Location::deserialize(&mut value)
          {
            self.largest_object = Some(loc);
          }
        }
        PARAM_AUTHORIZATION_TOKEN => {
          if let KeyValuePair::Bytes { value, .. } = param {
            self.authorization_token = Some(value);
          }
        }
        PARAM_SUBSCRIPTION_FILTER => {
          if let KeyValuePair::Bytes { mut value, .. } = param
            && let Ok(ft_raw) = value.get_vi()
            && let Ok(ft) = FilterType::try_from(ft_raw)
          {
            self.filter_type = ft;
            self.start_location = None;
            self.end_group = None;

            match ft {
              FilterType::AbsoluteStart => {
                if let Ok(loc) = Location::deserialize(&mut value) {
                  self.start_location = Some(loc);
                }
              }
              FilterType::AbsoluteRange => {
                if let Ok(loc) = Location::deserialize(&mut value) {
                  self.start_location = Some(loc);
                  if let Ok(eg) = value.get_vi() {
                    self.end_group = Some(eg);
                  }
                }
              }
              _ => {}
            }
          }
        }
        _ => {
          if let Some(existing) = self
            .unknown_parameters
            .iter_mut()
            .find(|p| p.get_type() == param.get_type())
          {
            *existing = param;
          } else {
            self.unknown_parameters.push(param);
          }
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use bytes::{Buf, BytesMut};

  #[test]
  fn test_roundtrip_message_parameters() {
    let mut params = MessageParameters::default();
    params.delivery_timeout = Some(0xABCD);
    params.subscriber_priority = 42;
    params.forward = false;
    params.filter_type = FilterType::AbsoluteStart;
    params.start_location = Some(Location {
      group: 10,
      object: 5,
    });
    params
      .unknown_parameters
      .push(KeyValuePair::try_new_varint(998, 1).unwrap());

    let kvps = params.clone().into_vec().unwrap();
    let mut buf = BytesMut::new();
    for kvp in &kvps {
      buf.extend_from_slice(&kvp.serialize().unwrap());
    }

    let mut bytes = buf.freeze();

    let deserialized = MessageParameters::deserialize(&mut bytes, kvps.len() as u64).unwrap();

    assert_eq!(params, deserialized);
    assert_eq!(bytes.remaining(), 0);
  }

  #[test]
  fn test_default_omission_serialization() {
    let params = MessageParameters::default();
    let kvps = params.into_vec().unwrap();

    // Should serialize entirely empty because everything is a default
    assert_eq!(kvps.len(), 0);
  }

  #[test]
  fn test_apply_update() {
    let mut params = MessageParameters::default();
    params.subscriber_priority = 100;

    let update_kvps = vec![
      KeyValuePair::try_new_varint(PARAM_SUBSCRIBER_PRIORITY, 50).unwrap(),
      KeyValuePair::try_new_varint(PARAM_FORWARD, 0).unwrap(),
    ];

    params.apply_update(update_kvps);

    assert_eq!(params.subscriber_priority, 50); // Updated
    assert!(!params.forward); // Updated
    assert_eq!(params.delivery_timeout, None); // Untouched
  }
}
