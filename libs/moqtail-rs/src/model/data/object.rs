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

use std::fmt;

use bytes::Bytes;

use crate::model::common::location::Location;
use crate::model::common::pair::KeyValuePair;
use crate::model::error::ParseError;

use super::constant::{ObjectForwardingPreference, ObjectStatus};
use super::datagram_object::DatagramObject;
use super::datagram_status::DatagramStatus;
use super::fetch_object::FetchObject;
use super::subgroup_object::SubgroupObject;

#[derive(Clone, PartialEq)]
pub struct Object {
  pub track_alias: u64,
  pub location: Location,
  pub publisher_priority: u8,
  pub forwarding_preference: ObjectForwardingPreference,
  pub subgroup_id: Option<u64>,
  pub status: ObjectStatus,
  pub extensions: Option<Vec<KeyValuePair>>,
  pub payload: Option<Bytes>,
}
impl fmt::Debug for Object {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("Object")
      .field("track_alias", &self.track_alias)
      .field("location", &self.location)
      .field("publisher_priority", &self.publisher_priority)
      .field("forwarding_preference", &self.forwarding_preference)
      .field("subgroup_id", &self.subgroup_id)
      .field("status", &self.status)
      .field("extensions", &self.extensions)
      .field("payload_length", &self.payload.as_ref().map(|p| p.len()))
      .finish()
  }
}

impl Object {
  /// Convert from DatagramObject.
  /// Returns (Object, end_of_group) since end_of_group is datagram-specific.
  pub fn try_from_datagram(datagram: DatagramObject) -> Result<(Self, bool), ParseError> {
    let end_of_group = datagram.end_of_group;
    Ok((
      Object {
        track_alias: datagram.track_alias,
        location: Location {
          group: datagram.group_id,
          object: datagram.object_id,
        },
        publisher_priority: datagram.publisher_priority,
        forwarding_preference: ObjectForwardingPreference::Datagram,
        subgroup_id: None, // Datagrams don't have subgroup ID in canonical form
        status: ObjectStatus::Normal, // Datagrams always imply Normal status
        extensions: datagram.extension_headers, // Directly use the parsed extensions
        payload: Some(datagram.payload),
      },
      end_of_group,
    ))
  }

  pub fn try_from_datagram_status(status_msg: DatagramStatus) -> Result<Self, ParseError> {
    Ok(Object {
      track_alias: status_msg.track_alias,
      location: Location {
        group: status_msg.group_id,
        object: status_msg.object_id,
      },
      publisher_priority: status_msg.publisher_priority,
      forwarding_preference: ObjectForwardingPreference::Datagram,
      subgroup_id: None,
      status: status_msg.object_status,
      extensions: status_msg.extension_headers,
      payload: None,
    })
  }

  pub fn try_from_subgroup(
    subgroup_obj: SubgroupObject,
    track_alias: u64,         // Context from SubgroupHeader
    group_id: u64,            // Context from SubgroupHeader
    subgroup_id: Option<u64>, // Context from SubgroupHeader
    publisher_priority: u8,   // Context from SubgroupHeader
  ) -> Result<Self, ParseError> {
    Ok(Object {
      track_alias,
      location: Location {
        group: group_id,
        object: subgroup_obj.object_id,
      },
      publisher_priority,
      forwarding_preference: ObjectForwardingPreference::Subgroup,
      subgroup_id,
      status: subgroup_obj.object_status.unwrap_or(ObjectStatus::Normal),
      extensions: subgroup_obj.extension_headers,
      payload: subgroup_obj.payload,
    })
  }

  pub fn try_from_fetch(fetch_obj: FetchObject, track_alias: u64) -> Result<Self, ParseError> {
    Ok(Object {
      track_alias, // Context from FetchHeader RequestId == Fetch Message Request Id, Fetch.track_alias
      location: Location {
        group: fetch_obj.group_id,
        object: fetch_obj.object_id,
      },
      publisher_priority: fetch_obj.publisher_priority,
      forwarding_preference: ObjectForwardingPreference::Subgroup,
      subgroup_id: Some(fetch_obj.subgroup_id),
      status: fetch_obj.object_status.unwrap_or(ObjectStatus::Normal),
      extensions: fetch_obj.extension_headers,
      payload: fetch_obj.payload,
    })
  }
}

impl Object {
  /// Convert to DatagramObject for wire transmission.
  ///
  /// # Arguments
  /// * `track_alias` - Track alias to use
  /// * `end_of_group` - Draft-14: Whether this is the last object in the group
  pub fn try_into_datagram(
    self,
    track_alias: u64,
    end_of_group: bool,
  ) -> Result<DatagramObject, ParseError> {
    if self.forwarding_preference != ObjectForwardingPreference::Datagram {
      return Err(ParseError::CastingError {
        context: "Object::try_into_datagram(forwarding_preference)",
        from_type: "Object",
        to_type: "ObjectDatagram",
        details: "Forwarding preference must be Datagram".to_string(),
      });
    }
    if self.status != ObjectStatus::Normal {
      return Err(ParseError::CastingError {
        context: "Object::try_into_datagram(status)",
        from_type: "Object",
        to_type: "ObjectDatagram",
        details: "Object status must be Normal".to_string(),
      });
    }
    let payload = self.payload.ok_or(ParseError::CastingError {
      context: "Object::try_into_datagram(payload)",
      from_type: "Object",
      to_type: "ObjectDatagram",
      details: "Payload cannot be empty".to_string(),
    })?;

    Ok(DatagramObject {
      track_alias,
      group_id: self.location.group,
      object_id: self.location.object,
      publisher_priority: self.publisher_priority,
      extension_headers: self.extensions,
      payload,
      end_of_group,
    })
  }

  pub fn try_into_datagram_status(self, track_alias: u64) -> Result<DatagramStatus, ParseError> {
    if self.forwarding_preference != ObjectForwardingPreference::Datagram {
      return Err(ParseError::CastingError {
        context: "Object::try_into_datagram_status(forwarding_preference)",
        from_type: "Object",
        to_type: "DatagramStatus",
        details: "Forwarding preference must be Datagram".to_string(),
      });
    }
    if self.status == ObjectStatus::Normal {
      return Err(ParseError::CastingError {
        context: "Object::try_into_datagram_status(status)",
        from_type: "Object",
        to_type: "DatagramStatus",
        details: "Object status must not be Normal".to_string(),
      });
    }
    if self.payload.is_some() {
      return Err(ParseError::CastingError {
        context: "Object::try_into_datagram_status(payload)",
        from_type: "Object",
        to_type: "DatagramStatus",
        details: "Payload must be None for non-Normal status".to_string(),
      });
    }

    Ok(DatagramStatus {
      track_alias,
      group_id: self.location.group,
      object_id: self.location.object,
      publisher_priority: self.publisher_priority,
      extension_headers: self.extensions,
      object_status: self.status,
    })
  }

  pub fn try_into_subgroup(self) -> Result<SubgroupObject, ParseError> {
    if self.forwarding_preference != ObjectForwardingPreference::Subgroup {
      return Err(ParseError::CastingError {
        context: "Object::try_into_subgroup(forwarding_preference)",
        from_type: "Object",
        to_type: "SubgroupObject",
        details: "Forwarding preference must be Subgroup".to_string(),
      });
    }
    let _subgroup_id = self.subgroup_id.ok_or(ParseError::CastingError {
      context: "Object::try_into_subgroup(subgroup_id)",
      from_type: "Object",
      to_type: "SubgroupObject",
      details: "Subgroup ID must be present for Subgroup forwarding".to_string(),
    })?;

    let (payload, object_status) = match self.status {
      ObjectStatus::Normal => (
        Some(self.payload.ok_or(ParseError::CastingError {
          context: "Object::try_into_subgroup(payload)",
          from_type: "Object",
          to_type: "SubgroupObject",
          details: "Payload must be present for Normal status".to_string(),
        })?),
        None,
      ),
      other_status => {
        if self.payload.is_some() {
          return Err(ParseError::CastingError {
            context: "Object::try_into_subgroup(payload)",
            from_type: "Object",
            to_type: "SubgroupObject",
            details: "Payload must be None for non-Normal status".to_string(),
          });
        }
        (None, Some(other_status))
      }
    };

    Ok(SubgroupObject {
      object_id: self.location.object,
      extension_headers: self.extensions,
      object_status,
      payload,
    })
  }

  pub fn try_into_fetch(self) -> Result<FetchObject, ParseError> {
    let (payload, object_status) = match self.status {
      ObjectStatus::Normal => (
        Some(self.payload.ok_or(ParseError::CastingError {
          context: "Object::try_into_fetch(payload)",
          from_type: "Object",
          to_type: "FetchObject",
          details: "Payload must be present for Normal status".to_string(),
        })?),
        None,
      ),
      other_status => {
        if self.payload.is_some() {
          return Err(ParseError::CastingError {
            context: "Object::try_into_fetch(payload)",
            from_type: "Object",
            to_type: "FetchObject",
            details: "Payload must be None for non-Normal status".to_string(),
          });
        }
        (None, Some(other_status))
      }
    };

    let subgroup_id = match self.forwarding_preference {
      ObjectForwardingPreference::Subgroup => self.subgroup_id.ok_or(ParseError::CastingError {
        context: "Object::try_into_fetch(subgroup_id)",
        from_type: "Object",
        to_type: "FetchObject",
        details: "Subgroup ID must be present for Subgroup forwarding".to_string(),
      })?,
      ObjectForwardingPreference::Datagram => {
        if self.subgroup_id.is_some() {
          return Err(ParseError::CastingError {
            context: "Object::try_into_fetch(subgroup_id)",
            from_type: "Object",
            to_type: "FetchObject",
            details: "Subgroup ID must not be present for Datagram forwarding".to_string(),
          });
        }
        self.location.object
      }
    };

    Ok(FetchObject {
      group_id: self.location.group,
      subgroup_id,
      object_id: self.location.object,
      publisher_priority: self.publisher_priority,
      extension_headers: self.extensions,
      payload,
      object_status,
    })
  }
}
