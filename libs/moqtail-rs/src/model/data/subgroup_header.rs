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
  pub publisher_priority: u8,
}

impl SubgroupHeader {
  /// Create a new subgroup header with fixed Subgroup ID = 0
  pub fn new_fixed_zero_id(
    track_alias: u64,
    group_id: u64,
    publisher_priority: u8,
    has_extensions: bool,
    contains_end_of_group: bool,
  ) -> Self {
    let header_type = match (has_extensions, contains_end_of_group) {
      (false, false) => SubgroupHeaderType::Type0x10,
      (true, false) => SubgroupHeaderType::Type0x11,
      (false, true) => SubgroupHeaderType::Type0x18,
      (true, true) => SubgroupHeaderType::Type0x19,
    };

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
    publisher_priority: u8,
    has_extensions: bool,
    contains_end_of_group: bool,
  ) -> Self {
    let header_type = match (has_extensions, contains_end_of_group) {
      (false, false) => SubgroupHeaderType::Type0x12,
      (true, false) => SubgroupHeaderType::Type0x13,
      (false, true) => SubgroupHeaderType::Type0x1A,
      (true, true) => SubgroupHeaderType::Type0x1B,
    };

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
    publisher_priority: u8,
    has_extensions: bool,
    contains_end_of_group: bool,
  ) -> Self {
    let header_type = match (has_extensions, contains_end_of_group) {
      (false, false) => SubgroupHeaderType::Type0x14,
      (true, false) => SubgroupHeaderType::Type0x15,
      (false, true) => SubgroupHeaderType::Type0x1C,
      (true, true) => SubgroupHeaderType::Type0x1D,
    };

    Self {
      header_type,
      track_alias,
      group_id,
      subgroup_id: Some(subgroup_id),
      publisher_priority,
    }
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();

    // Type field
    buf.put_vi(self.header_type as u64)?;

    // Track Alias
    buf.put_vi(self.track_alias)?;

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

    // Publisher Priority
    buf.put_u8(self.publisher_priority);

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

    if bytes.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "SubgroupHeader::deserialize(publisher_priority)",
        needed: 1,
        available: bytes.remaining(),
      });
    }
    let publisher_priority = bytes.get_u8();

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
    let header_type = SubgroupHeaderType::Type0x14;
    let track_alias = 87;
    let group_id = 9;
    let subgroup_id = Some(11);
    let publisher_priority = 255;
    let subgroup_header = SubgroupHeader {
      header_type,
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
    };

    let mut buf = subgroup_header.serialize().unwrap();
    let deserialized = SubgroupHeader::deserialize(&mut buf).unwrap();
    assert_eq!(deserialized, subgroup_header);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let header_type = SubgroupHeaderType::Type0x14;
    let track_alias = 87;
    let group_id = 9;
    let subgroup_id = Some(11);
    let publisher_priority = 255;
    let subgroup_header = SubgroupHeader {
      header_type,
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
    };

    let serialized = subgroup_header.serialize().unwrap();
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
    let header_type = SubgroupHeaderType::Type0x14;
    let track_alias = 87;
    let group_id = 9;
    let subgroup_id = Some(11);
    let publisher_priority = 255;
    let subgroup_header = SubgroupHeader {
      header_type,
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
    };
    let buf = subgroup_header.serialize().unwrap();
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
      (SubgroupHeaderType::Type0x10, false, Some(0)),
      (SubgroupHeaderType::Type0x11, false, Some(0)),
      (SubgroupHeaderType::Type0x12, false, None),
      (SubgroupHeaderType::Type0x13, false, None),
      (SubgroupHeaderType::Type0x14, true, Some(42)),
      (SubgroupHeaderType::Type0x15, true, Some(42)),
      (SubgroupHeaderType::Type0x18, false, Some(0)),
      (SubgroupHeaderType::Type0x19, false, Some(0)),
      (SubgroupHeaderType::Type0x1A, false, None),
      (SubgroupHeaderType::Type0x1B, false, None),
      (SubgroupHeaderType::Type0x1C, true, Some(42)),
      (SubgroupHeaderType::Type0x1D, true, Some(42)),
    ];

    for (header_type, has_explicit_id, expected_subgroup_id) in test_cases {
      let track_alias = 87;
      let group_id = 9;
      let publisher_priority = 255;

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

      let mut buf = subgroup_header.serialize().unwrap();
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
    assert!(SubgroupHeaderType::Type0x10.subgroup_id_is_zero());
    assert!(SubgroupHeaderType::Type0x11.subgroup_id_is_zero());
    assert!(SubgroupHeaderType::Type0x18.subgroup_id_is_zero());
    assert!(SubgroupHeaderType::Type0x19.subgroup_id_is_zero());

    // Test subgroup_id_is_first_object_id
    assert!(SubgroupHeaderType::Type0x12.subgroup_id_is_first_object_id());
    assert!(SubgroupHeaderType::Type0x13.subgroup_id_is_first_object_id());
    assert!(SubgroupHeaderType::Type0x1A.subgroup_id_is_first_object_id());
    assert!(SubgroupHeaderType::Type0x1B.subgroup_id_is_first_object_id());

    // Test has_explicit_subgroup_id
    assert!(SubgroupHeaderType::Type0x14.has_explicit_subgroup_id());
    assert!(SubgroupHeaderType::Type0x15.has_explicit_subgroup_id());
    assert!(SubgroupHeaderType::Type0x1C.has_explicit_subgroup_id());
    assert!(SubgroupHeaderType::Type0x1D.has_explicit_subgroup_id());

    // Test contains_end_of_group
    assert!(SubgroupHeaderType::Type0x18.contains_end_of_group());
    assert!(SubgroupHeaderType::Type0x19.contains_end_of_group());
    assert!(SubgroupHeaderType::Type0x1A.contains_end_of_group());
    assert!(SubgroupHeaderType::Type0x1B.contains_end_of_group());
    assert!(SubgroupHeaderType::Type0x1C.contains_end_of_group());
    assert!(SubgroupHeaderType::Type0x1D.contains_end_of_group());

    // Test has_extensions
    assert!(SubgroupHeaderType::Type0x11.has_extensions());
    assert!(SubgroupHeaderType::Type0x13.has_extensions());
    assert!(SubgroupHeaderType::Type0x15.has_extensions());
    assert!(SubgroupHeaderType::Type0x19.has_extensions());
    assert!(SubgroupHeaderType::Type0x1B.has_extensions());
    assert!(SubgroupHeaderType::Type0x1D.has_extensions());
  }

  #[test]
  fn test_constructor_methods() {
    let track_alias = 87;
    let group_id = 9;
    let publisher_priority = 255;

    // Test new_fixed_zero_id
    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, false, false);
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x10);
    assert_eq!(header.subgroup_id, Some(0));

    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, true, false);
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x11);
    assert_eq!(header.subgroup_id, Some(0));

    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, false, true);
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x18);
    assert_eq!(header.subgroup_id, Some(0));

    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, true, true);
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x19);
    assert_eq!(header.subgroup_id, Some(0));

    // Test new_first_object_id
    let header =
      SubgroupHeader::new_first_object_id(track_alias, group_id, publisher_priority, false, false);
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x12);
    assert_eq!(header.subgroup_id, None);

    let header =
      SubgroupHeader::new_first_object_id(track_alias, group_id, publisher_priority, true, false);
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x13);
    assert_eq!(header.subgroup_id, None);

    let header =
      SubgroupHeader::new_first_object_id(track_alias, group_id, publisher_priority, false, true);
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x1A);
    assert_eq!(header.subgroup_id, None);

    let header =
      SubgroupHeader::new_first_object_id(track_alias, group_id, publisher_priority, true, true);
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x1B);
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
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x14);
    assert_eq!(header.subgroup_id, Some(subgroup_id));

    let header = SubgroupHeader::new_with_explicit_id(
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
      true,
      false,
    );
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x15);
    assert_eq!(header.subgroup_id, Some(subgroup_id));

    let header = SubgroupHeader::new_with_explicit_id(
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
      false,
      true,
    );
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x1C);
    assert_eq!(header.subgroup_id, Some(subgroup_id));

    let header = SubgroupHeader::new_with_explicit_id(
      track_alias,
      group_id,
      subgroup_id,
      publisher_priority,
      true,
      true,
    );
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x1D);
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
    let publisher_priority = 255;

    // Test a header type with fixed subgroup ID = 0
    let header =
      SubgroupHeader::new_fixed_zero_id(track_alias, group_id, publisher_priority, false, false);
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x10);
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
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x15);
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
    assert_eq!(header.header_type, SubgroupHeaderType::Type0x1A);
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
