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

use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::constant::SubgroupHeaderType;

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct SubgroupHeader {
  pub header_type: SubgroupHeaderType,
  pub track_alias: u64,
  pub group_id: u64,
  pub subgroup_id: Option<u64>,
  /// Publisher priority. None when header_type has DEFAULT_PRIORITY bit set.
  pub publisher_priority: Option<u8>,
}

impl SubgroupHeader {
  /// Create a new subgroup header with fixed Subgroup ID = 0
  pub fn new_fixed_zero_id(
    track_alias: u64,
    group_id: u64,
    publisher_priority: Option<u8>,
    has_extensions: bool,
    contains_end_of_group: bool,
  ) -> Self {
    let has_default_priority = publisher_priority.is_none();
    let header_type = SubgroupHeaderType::from_properties(
      has_extensions,
      0,
      contains_end_of_group,
      has_default_priority,
    );

    Self {
      header_type,
      track_alias,
      group_id,
      subgroup_id: Some(0),
      publisher_priority,
    }
  }

  /// Create a new subgroup header where Subgroup ID = First Object ID
  pub fn new_first_object_id(
    track_alias: u64,
    group_id: u64,
    publisher_priority: Option<u8>,
    has_extensions: bool,
    contains_end_of_group: bool,
  ) -> Self {
    let has_default_priority = publisher_priority.is_none();
    let header_type = SubgroupHeaderType::from_properties(
      has_extensions,
      1,
      contains_end_of_group,
      has_default_priority,
    );

    Self {
      header_type,
      track_alias,
      group_id,
      subgroup_id: None, // Will be set to first object ID
      publisher_priority,
    }
  }

  /// Create a new subgroup header with explicit Subgroup ID
  pub fn new_with_explicit_id(
    track_alias: u64,
    group_id: u64,
    subgroup_id: u64,
    publisher_priority: Option<u8>,
    has_extensions: bool,
    contains_end_of_group: bool,
  ) -> Self {
    let has_default_priority = publisher_priority.is_none();
    let header_type = SubgroupHeaderType::from_properties(
      has_extensions,
      2,
      contains_end_of_group,
      has_default_priority,
    );

    Self {
      header_type,
      track_alias,
      group_id,
      subgroup_id: Some(subgroup_id),
      publisher_priority,
    }
  }

  pub fn serialize(&self, track_alias: Option<u64>) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();

    // Type field
    buf.put_vi(u64::from(self.header_type))?;

    // Track Alias may be set by the subscription for multi-publisher cases
    if track_alias.is_none() {
      buf.put_vi(self.track_alias)?;
    } else {
      buf.put_vi(track_alias.unwrap())?;
    }

    // Group ID
    buf.put_vi(self.group_id)?;

    // Subgroup ID (if present)
    if self.header_type.has_explicit_subgroup_id() {
      if let Some(id) = self.subgroup_id {
        buf.put_vi(id)?;
      } else {
        return Err(ParseError::ProtocolViolation {
          context: "SubgroupHeader::serialize(header_type)",
          details: "Subgroup_id field is required for this header type".to_string(),
        });
      }
    }

    // Publisher Priority (omitted when DEFAULT_PRIORITY bit is set)
    if !self.header_type.has_default_priority() {
      if let Some(priority) = self.publisher_priority {
        buf.put_u8(priority);
      } else {
        return Err(ParseError::ProtocolViolation {
          context: "SubgroupHeader::serialize(publisher_priority)",
          details: "Publisher_priority field is required when DEFAULT_PRIORITY bit is not set"
            .to_string(),
        });
      }
    }

    Ok(buf.freeze())
  }

  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    // Parse type
    let type_value = bytes.get_vi()?;

    let header_type = SubgroupHeaderType::try_from(type_value)?;

    // Parse track alias
    let track_alias = bytes.get_vi()?;

    // Parse group ID
    let group_id = bytes.get_vi()?;

    // Parse subgroup ID if present
    let subgroup_id = if header_type.has_explicit_subgroup_id() {
      Some(bytes.get_vi()?)
    } else if header_type.subgroup_id_is_zero() {
      Some(0) // Fixed at 0 for types that specify Subgroup ID = 0
    } else {
      None // For types where Subgroup ID = first object ID
    };

    // Parse Publisher Priority (omitted when DEFAULT_PRIORITY bit is set)
    let publisher_priority = if !header_type.has_default_priority() {
      if bytes.remaining() < 1 {
        return Err(ParseError::NotEnoughBytes {
          context: "SubgroupHeader::deserialize(publisher_priority)",
          needed: 1,
          available: bytes.remaining(),
        });
      }
      Some(bytes.get_u8())
    } else {
      None
    };

    Ok(Self {
      header_type,
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
    })
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let header_type = SubgroupHeaderType::try_new(0x14).unwrap();
    let track_alias = 87;
    let group_id = 9;
    let subgroup_id = Some(11);
    let publisher_priority = Some(255);
    let subgroup_header = SubgroupHeader {
      header_type,
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
    };

    let mut buf = subgroup_header.serialize(None).unwrap();
    let deserialized = SubgroupHeader::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, subgroup_header);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_default_priority() {
    let header_type = SubgroupHeaderType::try_new(0x30).unwrap(); // DEFAULT_PRIORITY bit set
    let track_alias = 87;
    let group_id = 9;
    let subgroup_id = Some(0);
    let publisher_priority = None; // Not included in wire format
    let subgroup_header = SubgroupHeader {
      header_type,
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
    };

    let mut buf = subgroup_header.serialize(None).unwrap();
    let deserialized = SubgroupHeader::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, subgroup_header);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let header_type = SubgroupHeaderType::try_new(0x14).unwrap();
    let track_alias = 87;
    let group_id = 9;
    let subgroup_id = Some(11);
    let publisher_priority = Some(255);
    let subgroup_header = SubgroupHeader {
      header_type,
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
    };

    let serialized = subgroup_header.serialize(None).unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let deserialized = SubgroupHeader::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, subgroup_header);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let header_type = SubgroupHeaderType::try_new(0x14).unwrap();
    let track_alias = 87;
    let group_id = 9;
    let subgroup_id = Some(11);
    let publisher_priority = Some(255);
    let subgroup_header = SubgroupHeader {
      header_type,
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
    };
    let buf = subgroup_header.serialize(None).unwrap();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = SubgroupHeader::deserialize(&mut partial);
    assert!(deserialized.is_err());
  }

  #[test]
  fn test_new_header_types_roundtrip() {
    // Test all new header types (0x10-0x1D)
    let test_cases = vec![
      // (header_type, has_explicit_id, expected_subgroup_id)
      (SubgroupHeaderType::try_new(0x10).unwrap(), false, Some(0)),
      (SubgroupHeaderType::try_new(0x11).unwrap(), false, Some(0)),
      (SubgroupHeaderType::try_new(0x12).unwrap(), false, None),
      (SubgroupHeaderType::try_new(0x13).unwrap(), false, None),
      (SubgroupHeaderType::try_new(0x14).unwrap(), true, Some(42)),
      (SubgroupHeaderType::try_new(0x15).unwrap(), true, Some(42)),
      (SubgroupHeaderType::try_new(0x18).unwrap(), false, Some(0)),
      (SubgroupHeaderType::try_new(0x19).unwrap(), false, Some(0)),
      (SubgroupHeaderType::try_new(0x1A).unwrap(), false, None),
      (SubgroupHeaderType::try_new(0x1B).unwrap(), false, None),
      (SubgroupHeaderType::try_new(0x1C).unwrap(), true, Some(42)),
      (SubgroupHeaderType::try_new(0x1D).unwrap(), true, Some(42)),
    ];

    for (header_type, has_explicit_id, expected_subgroup_id) in test_cases {
      let track_alias = 87;
      let group_id = 9;
      let publisher_priority = Some(255);

      let subgroup_header = SubgroupHeader {
        header_type,
        track_alias,
        group_id,
        subgroup_id: if has_explicit_id {
          Some(42)
        } else {
          expected_subgroup_id
        },
        publisher_priority,
      };

      let mut buf = subgroup_header.serialize(Some(track_alias)).unwrap();
      let deserialized = SubgroupHeader::deserialize(&mut buf).unwrap();

      assert_eq!(deserialized.header_type, header_type);
      assert_eq!(deserialized.track_alias, track_alias);
      assert_eq!(deserialized.group_id, group_id);
      assert_eq!(deserialized.subgroup_id, expected_subgroup_id);
      assert_eq!(deserialized.publisher_priority, publisher_priority);
      assert!(!buf.has_remaining());
    }
  }

  #[test]
  fn test_header_type_classification() {
    // Test subgroup_id_is_zero
    assert!(
      SubgroupHeaderType::try_new(0x10)
        .unwrap()
        .subgroup_id_is_zero()
    );
    assert!(
      SubgroupHeaderType::try_new(0x11)
        .unwrap()
        .subgroup_id_is_zero()
    );
    assert!(
      SubgroupHeaderType::try_new(0x18)
        .unwrap()
        .subgroup_id_is_zero()
    );
    assert!(
      SubgroupHeaderType::try_new(0x19)
        .unwrap()
        .subgroup_id_is_zero()
    );

    // Test subgroup_id_is_first_object_id
    assert!(
      SubgroupHeaderType::try_new(0x12)
        .unwrap()
        .subgroup_id_is_first_object_id()
    );
    assert!(
      SubgroupHeaderType::try_new(0x13)
        .unwrap()
        .subgroup_id_is_first_object_id()
    );
    assert!(
      SubgroupHeaderType::try_new(0x1A)
        .unwrap()
        .subgroup_id_is_first_object_id()
    );
    assert!(
      SubgroupHeaderType::try_new(0x1B)
        .unwrap()
        .subgroup_id_is_first_object_id()
    );

    // Test has_explicit_subgroup_id
    assert!(
      SubgroupHeaderType::try_new(0x14)
        .unwrap()
        .has_explicit_subgroup_id()
    );
    assert!(
      SubgroupHeaderType::try_new(0x15)
        .unwrap()
        .has_explicit_subgroup_id()
    );
    assert!(
      SubgroupHeaderType::try_new(0x1C)
        .unwrap()
        .has_explicit_subgroup_id()
    );
    assert!(
      SubgroupHeaderType::try_new(0x1D)
        .unwrap()
        .has_explicit_subgroup_id()
    );

    // Test contains_end_of_group
    assert!(
      SubgroupHeaderType::try_new(0x18)
        .unwrap()
        .contains_end_of_group()
    );
    assert!(
      SubgroupHeaderType::try_new(0x19)
        .unwrap()
        .contains_end_of_group()
    );
    assert!(
      SubgroupHeaderType::try_new(0x1A)
        .unwrap()
        .contains_end_of_group()
    );
    assert!(
      SubgroupHeaderType::try_new(0x1B)
        .unwrap()
        .contains_end_of_group()
    );
    assert!(
      SubgroupHeaderType::try_new(0x1C)
        .unwrap()
        .contains_end_of_group()
    );
    assert!(
      SubgroupHeaderType::try_new(0x1D)
        .unwrap()
        .contains_end_of_group()
    );

    // Test has_extensions
    assert!(SubgroupHeaderType::try_new(0x11).unwrap().has_extensions());
    assert!(SubgroupHeaderType::try_new(0x13).unwrap().has_extensions());
    assert!(SubgroupHeaderType::try_new(0x15).unwrap().has_extensions());
    assert!(SubgroupHeaderType::try_new(0x19).unwrap().has_extensions());
    assert!(SubgroupHeaderType::try_new(0x1B).unwrap().has_extensions());
    assert!(SubgroupHeaderType::try_new(0x1D).unwrap().has_extensions());
  }

  #[test]
  fn test_constructor_methods() {
    let track_alias = 87;
    let group_id = 9;
    let publisher_priority = Some(255);

    // Test new_fixed_zero_id
    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, false, false);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x10).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(0));

    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, true, false);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x11).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(0));

    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, false, true);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x18).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(0));

    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, true, true);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x19).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(0));

    // Test new_first_object_id
    let header =
      SubgroupHeader::new_first_object_id(track_alias, group_id, publisher_priority, false, false);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x12).unwrap()
    );
    assert_eq!(header.subgroup_id, None);

    let header =
      SubgroupHeader::new_first_object_id(track_alias, group_id, publisher_priority, true, false);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x13).unwrap()
    );
    assert_eq!(header.subgroup_id, None);

    let header =
      SubgroupHeader::new_first_object_id(track_alias, group_id, publisher_priority, false, true);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x1A).unwrap()
    );
    assert_eq!(header.subgroup_id, None);

    let header =
      SubgroupHeader::new_first_object_id(track_alias, group_id, publisher_priority, true, true);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x1B).unwrap()
    );
    assert_eq!(header.subgroup_id, None);

    // Test new_with_explicit_id
    let subgroup_id = 42;
    let header = SubgroupHeader::new_with_explicit_id(
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
      false,
      false,
    );
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x14).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(subgroup_id));

    let header = SubgroupHeader::new_with_explicit_id(
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
      true,
      false,
    );
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x15).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(subgroup_id));

    let header = SubgroupHeader::new_with_explicit_id(
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
      false,
      true,
    );
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x1C).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(subgroup_id));

    let header = SubgroupHeader::new_with_explicit_id(
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
      true,
      true,
    );
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x1D).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(subgroup_id));
  }

  #[test]
  fn test_object_parsing_integration() {
    // Test that new header types work correctly with object parsing
    use crate::model::data::object::Object;
    use crate::model::data::subgroup_object::SubgroupObject;
    use bytes::Bytes;

    let track_alias = 87;
    let group_id = 9;
    let publisher_priority = Some(255);

    // Test a header type with fixed subgroup ID = 0
    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, false, false);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x10).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(0));

    // Create a subgroup object
    let subgroup_obj = SubgroupObject {
      object_id: 42,
      extension_headers: None,
      payload: Some(Bytes::from_static(b"test payload")),
      object_status: None,
    };

    // Convert to object using header context
    let object = Object::try_from_subgroup(
      subgroup_obj,
      header.track_alias,
      header.group_id,
      header.subgroup_id,
      header.publisher_priority,
    )
    .unwrap();

    // Verify the object has the correct subgroup_id from header
    assert_eq!(object.subgroup_id, Some(0));
    assert_eq!(object.track_alias, track_alias);
    assert_eq!(object.location.group, group_id);
    assert_eq!(object.location.object, 42);

    // Test a header type with explicit subgroup ID
    let header = SubgroupHeader::new_with_explicit_id(
      track_alias,
      group_id,
      123,
      publisher_priority,
      true,
      false,
    );
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x15).unwrap()
    );
    assert_eq!(header.subgroup_id, Some(123));

    let object = Object::try_from_subgroup(
      SubgroupObject {
        object_id: 43,
        extension_headers: None,
        payload: Some(Bytes::from_static(b"test payload 2")),
        object_status: None,
      },
      header.track_alias,
      header.group_id,
      header.subgroup_id,
      header.publisher_priority,
    )
    .unwrap();

    // Verify the object has the correct explicit subgroup_id
    assert_eq!(object.subgroup_id, Some(123));

    // Test a header type where subgroup ID = first object ID
    let header =
      SubgroupHeader::new_first_object_id(track_alias, group_id, publisher_priority, false, true);
    assert_eq!(
      header.header_type,
      SubgroupHeaderType::try_new(0x1A).unwrap()
    );
    assert_eq!(header.subgroup_id, None);

    let object = Object::try_from_subgroup(
      SubgroupObject {
        object_id: 55,
        extension_headers: None,
        payload: Some(Bytes::from_static(b"test payload 3")),
        object_status: None,
      },
      header.track_alias,
      header.group_id,
      header.subgroup_id, // This should be None, meaning subgroup_id = first object ID
      header.publisher_priority,
    )
    .unwrap();

    // For first object ID headers, subgroup_id in the object should be None
    // (the actual subgroup ID would be determined by the first object ID during streaming)
    assert_eq!(object.subgroup_id, None);
  }
}
