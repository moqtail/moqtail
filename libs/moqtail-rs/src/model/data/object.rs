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
use crate::model::error::ParseError;
use crate::model::extension_header::object_extension::ObjectExtension;

use super::constant::{ObjectForwardingPreference, ObjectStatus};
use super::datagram::Datagram;
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
  pub extensions: Option<Vec<ObjectExtension>>,
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
  /// Convert from a Datagram object.
  /// Returns (Object, end_of_group) since end_of_group is datagram-specific.
  /// When `publisher_priority` is None (DEFAULT_PRIORITY), `default_priority` is used.
  pub fn try_from_datagram(
    datagram: Datagram,
    default_priority: u8,
  ) -> Result<(Self, bool), ParseError> {
    let end_of_group = datagram.end_of_group;
    let priority = datagram.publisher_priority.unwrap_or(default_priority);

    let (status, payload) = if let Some(object_status) = datagram.object_status {
      (object_status, None)
    } else {
      (ObjectStatus::Normal, datagram.payload)
    };

    Ok((
      Object {
        track_alias: datagram.track_alias,
        location: Location {
          group: datagram.group_id,
          object: datagram.object_id,
        },
        publisher_priority: priority,
        forwarding_preference: ObjectForwardingPreference::Datagram,
        subgroup_id: None,
        status,
        extensions: datagram.extension_headers,
        payload,
      },
      end_of_group,
    ))
  }

  pub fn try_from_subgroup(
    subgroup_obj: SubgroupObject,
    track_alias: u64,               // Context from SubgroupHeader
    group_id: u64,                  // Context from SubgroupHeader
    subgroup_id: Option<u64>,       // Context from SubgroupHeader
    publisher_priority: Option<u8>, // Context from SubgroupHeader; None when using DEFAULT_PRIORITY
  ) -> Result<Self, ParseError> {
    Ok(Object {
      track_alias,
      location: Location {
        group: group_id,
        object: subgroup_obj.object_id,
      },
      publisher_priority: publisher_priority.unwrap_or(0), // Default to 0 if not specified
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
  /// Convert to a Datagram object for wire transmission.
  ///
  /// # Arguments
  /// * `track_alias` - Track alias to use
  /// * `end_of_group` - Whether this is the last object in the group
  /// * `default_priority` - If provided and matches self.publisher_priority, omit priority on wire (DEFAULT_PRIORITY bit)
  pub fn try_into_datagram(
    self,
    track_alias: u64,
    end_of_group: bool,
    default_priority: Option<u8>,
  ) -> Result<Datagram, ParseError> {
    if self.forwarding_preference != ObjectForwardingPreference::Datagram {
      return Err(ParseError::CastingError {
        context: "Object::try_into_datagram(forwarding_preference)",
        from_type: "Object",
        to_type: "Datagram",
        details: "Forwarding preference must be Datagram".to_string(),
      });
    }

    // Determine whether to omit priority on wire
    let publisher_priority =
      if default_priority.is_some() && default_priority == Some(self.publisher_priority) {
        None
      } else {
        Some(self.publisher_priority)
      };

    if self.status != ObjectStatus::Normal {
      // Status datagram
      if self.payload.is_some() {
        return Err(ParseError::CastingError {
          context: "Object::try_into_datagram(payload)",
          from_type: "Object",
          to_type: "Datagram",
          details: "Payload must be None for non-Normal status".to_string(),
        });
      }
      Ok(Datagram::new_status(
        track_alias,
        self.location.group,
        self.location.object,
        publisher_priority,
        self.extensions,
        self.status,
      ))
    } else {
      // Payload datagram
      let payload = self.payload.ok_or(ParseError::CastingError {
        context: "Object::try_into_datagram(payload)",
        from_type: "Object",
        to_type: "Datagram",
        details: "Payload cannot be empty".to_string(),
      })?;
      Ok(Datagram::new_payload(
        track_alias,
        self.location.group,
        self.location.object,
        publisher_priority,
        self.extensions,
        payload,
        end_of_group,
      ))
    }
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
