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
use crate::model::control::constant::GroupOrder;
use crate::model::error::ParseError;
use crate::model::property::object_property::{
  ObjectProperty, deserialize_object_properties, serialize_object_properties,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::constant::ObjectForwardingPreference;

/// Draft-18 §11.4.4 Serialization Flags bit layout.
const FLAG_SUBGROUP_MODE_MASK: u8 = 0x03;
const FLAG_OBJECT_ID_PRESENT: u8 = 0x04;
const FLAG_GROUP_ID_PRESENT: u8 = 0x08;
const FLAG_PRIORITY_PRESENT: u8 = 0x10;
const FLAG_PROPERTIES_PRESENT: u8 = 0x20;
const FLAG_DATAGRAM: u8 = 0x40;

const SUBGROUP_MODE_ZERO: u8 = 0b00;
const SUBGROUP_MODE_PRIOR: u8 = 0b01;
const SUBGROUP_MODE_PRIOR_PLUS_ONE: u8 = 0b10;
const SUBGROUP_MODE_PRESENT: u8 = 0b11;

/// Special Serialization Flag varint values for End-of-Range markers.
const END_OF_NON_EXISTENT_RANGE: u64 = 0x8C;
const END_OF_UNKNOWN_RANGE: u64 = 0x10C;

/// Prior-object state threaded across successive Fetch Objects on the same stream.
/// Used by both `serialize` and `deserialize` to compute / resolve inherited fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchObjectContext {
  pub group_id: u64,
  pub subgroup_id: u64,
  pub object_id: u64,
  pub publisher_priority: u8,
}

/// Kind discriminator for End-of-Range markers (§10.4.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndOfRangeKind {
  /// Serialization Flags = 0x8C: all objects in the range are known not to exist.
  NonExistent,
  /// Serialization Flags = 0x10C: the range is cache-unknown.
  Unknown,
}

impl EndOfRangeKind {
  fn as_flag_value(self) -> u64 {
    match self {
      EndOfRangeKind::NonExistent => END_OF_NON_EXISTENT_RANGE,
      EndOfRangeKind::Unknown => END_OF_UNKNOWN_RANGE,
    }
  }
}

/// Payload-bearing fetch object (§10.4.4, non-End-of-Range form).
#[derive(Debug, Clone, PartialEq)]
pub struct FetchObjectPayload {
  pub group_id: u64,
  pub subgroup_id: u64,
  pub object_id: u64,
  pub publisher_priority: u8,
  pub forwarding_preference: ObjectForwardingPreference,
  pub properties: Option<Vec<ObjectProperty>>,
  /// Draft-16 §10.4.4 FETCH objects carry no Object Status field; zero-length
  /// payload signals a zero-length Normal object.
  pub payload: Bytes,
}

/// A single object or marker on a FETCH data stream.
#[derive(Debug, Clone, PartialEq)]
pub enum FetchObject {
  Object(FetchObjectPayload),
  EndOfRange {
    kind: EndOfRangeKind,
    group_id: u64,
    object_id: u64,
  },
}

impl FetchObject {
  /// Context to thread into the next call on this stream.
  /// Returns None for EndOfRange markers — they MUST NOT update prior state.
  pub fn context(&self) -> Option<FetchObjectContext> {
    match self {
      FetchObject::Object(p) => Some(FetchObjectContext {
        group_id: p.group_id,
        subgroup_id: p.subgroup_id,
        object_id: p.object_id,
        publisher_priority: p.publisher_priority,
      }),
      FetchObject::EndOfRange { .. } => None,
    }
  }

  pub fn serialize(
    &self,
    prev: Option<&FetchObjectContext>,
    group_order: GroupOrder,
  ) -> Result<Bytes, ParseError> {
    match self {
      FetchObject::EndOfRange {
        kind,
        group_id,
        object_id,
      } => {
        let mut buf = BytesMut::new();
        buf.put_vi(kind.as_flag_value())?;
        buf.put_vi(*group_id)?;
        buf.put_vi(*object_id)?;
        Ok(buf.freeze())
      }
      FetchObject::Object(p) => serialize_payload(p, prev, group_order),
    }
  }

  pub fn deserialize(
    bytes: &mut Bytes,
    prev: Option<&FetchObjectContext>,
    group_order: GroupOrder,
  ) -> Result<Self, ParseError> {
    let flags_raw = bytes.get_vi()?;

    if flags_raw >= 128 {
      let kind = match flags_raw {
        END_OF_NON_EXISTENT_RANGE => EndOfRangeKind::NonExistent,
        END_OF_UNKNOWN_RANGE => EndOfRangeKind::Unknown,
        other => {
          return Err(ParseError::ProtocolViolation {
            context: "FetchObject::deserialize",
            details: format!("invalid Serialization Flags value {other:#x}"),
          });
        }
      };
      let group_id = bytes.get_vi()?;
      let object_id = bytes.get_vi()?;
      return Ok(FetchObject::EndOfRange {
        kind,
        group_id,
        object_id,
      });
    }

    let flags = flags_raw as u8;
    let subgroup_mode = flags & FLAG_SUBGROUP_MODE_MASK;
    let has_object_id = flags & FLAG_OBJECT_ID_PRESENT != 0;
    let has_group_id = flags & FLAG_GROUP_ID_PRESENT != 0;
    let has_priority = flags & FLAG_PRIORITY_PRESENT != 0;
    let has_properties = flags & FLAG_PROPERTIES_PRESENT != 0;
    let is_datagram = flags & FLAG_DATAGRAM != 0;

    // First object on the stream must not reference prior state.
    if prev.is_none() {
      if !has_object_id || !has_group_id || !has_priority {
        return Err(ParseError::ProtocolViolation {
          context: "FetchObject::deserialize",
          details: "first object must carry explicit group/object/priority".to_string(),
        });
      }
      if !is_datagram
        && (subgroup_mode == SUBGROUP_MODE_PRIOR || subgroup_mode == SUBGROUP_MODE_PRIOR_PLUS_ONE)
      {
        return Err(ParseError::ProtocolViolation {
          context: "FetchObject::deserialize",
          details: "first object cannot reference prior subgroup".to_string(),
        });
      }
    }

    // Group ID Delta present. For the first Object (no prior) the delta is the
    // absolute Group ID; otherwise it is applied to the prior Group ID per the
    // Group Order (draft-18 §11.4.4.1). A computed value outside [0, 2^64-1] is a
    // PROTOCOL_VIOLATION.
    let group_id = if has_group_id {
      let delta = bytes.get_vi()?;
      match prev {
        None => delta,
        Some(pc) => {
          let computed = match group_order {
            GroupOrder::Descending => delta
              .checked_add(1)
              .and_then(|step| pc.group_id.checked_sub(step)),
            GroupOrder::Ascending | GroupOrder::Original => pc
              .group_id
              .checked_add(delta)
              .and_then(|v| v.checked_add(1)),
          };
          computed.ok_or(ParseError::ProtocolViolation {
            context: "FetchObject::deserialize",
            details: format!(
              "group id delta wraps: prior={} delta={} order={group_order:?}",
              pc.group_id, delta
            ),
          })?
        }
      }
    } else {
      prev
        .ok_or(ParseError::ProtocolViolation {
          context: "FetchObject::deserialize",
          details: "group_id inherited but no prior object".to_string(),
        })?
        .group_id
    };

    let subgroup_id = if is_datagram {
      // 0x40: subgroup bits are ignored. In-memory subgroup_id is synthesized
      // from the object_id (resolved below) since datagram objects have none.
      0u64
    } else {
      match subgroup_mode {
        SUBGROUP_MODE_ZERO => 0,
        SUBGROUP_MODE_PRIOR => {
          prev
            .ok_or(ParseError::ProtocolViolation {
              context: "FetchObject::deserialize",
              details: "subgroup_id inherited but no prior object".to_string(),
            })?
            .subgroup_id
        }
        SUBGROUP_MODE_PRIOR_PLUS_ONE => {
          prev
            .ok_or(ParseError::ProtocolViolation {
              context: "FetchObject::deserialize",
              details: "subgroup_id inherited but no prior object".to_string(),
            })?
            .subgroup_id
            + 1
        }
        SUBGROUP_MODE_PRESENT => bytes.get_vi()?,
        _ => unreachable!("2-bit field"),
      }
    };

    // Object ID resolution (draft-18 §11.4.4.1):
    // - Object ID Delta present + Group ID Delta present (or first Object): the
    //   Object ID is the delta itself (absolute).
    // - Object ID Delta present + Group ID Delta absent: prior Object ID + delta.
    // - Object ID Delta absent: prior Object ID + 1.
    // A computed value greater than 2^64-1 is a PROTOCOL_VIOLATION.
    let object_id = if has_object_id {
      let delta = bytes.get_vi()?;
      match prev {
        None => delta,
        Some(_) if has_group_id => delta,
        Some(pc) => pc.object_id.checked_add(delta).ok_or({
          ParseError::ProtocolViolation {
            context: "FetchObject::deserialize",
            details: format!(
              "object id delta wraps: prior={} delta={delta}",
              pc.object_id
            ),
          }
        })?,
      }
    } else {
      let pc = prev.ok_or(ParseError::ProtocolViolation {
        context: "FetchObject::deserialize",
        details: "object_id inherited but no prior object".to_string(),
      })?;
      pc.object_id
        .checked_add(1)
        .ok_or(ParseError::ProtocolViolation {
          context: "FetchObject::deserialize",
          details: format!("object id wraps: prior={} +1", pc.object_id),
        })?
    };

    let publisher_priority = if has_priority {
      if bytes.remaining() < 1 {
        return Err(ParseError::NotEnoughBytes {
          context: "FetchObject::deserialize(priority)",
          needed: 1,
          available: 0,
        });
      }
      bytes.get_u8()
    } else {
      prev
        .ok_or(ParseError::ProtocolViolation {
          context: "FetchObject::deserialize",
          details: "priority inherited but no prior object".to_string(),
        })?
        .publisher_priority
    };

    let properties = if has_properties {
      let ext_len = bytes.get_vi()?;
      let ext_len: usize =
        ext_len
          .try_into()
          .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
            context: "FetchObject::deserialize(ext_len)",
            from_type: "u64",
            to_type: "usize",
            details: e.to_string(),
          })?;
      if bytes.remaining() < ext_len {
        return Err(ParseError::NotEnoughBytes {
          context: "FetchObject::deserialize(properties)",
          needed: ext_len,
          available: bytes.remaining(),
        });
      }
      let mut header_bytes = bytes.copy_to_bytes(ext_len);
      let headers = deserialize_object_properties(&mut header_bytes).map_err(|_| {
        ParseError::ProtocolViolation {
          context: "FetchObject::deserialize(properties)",
          details: "cannot parse object properties".to_string(),
        }
      })?;
      Some(headers)
    } else {
      None
    };

    let payload_len = bytes.get_vi()?;
    let payload_len: usize = payload_len
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "FetchObject::deserialize(payload_len)",
        from_type: "u64",
        to_type: "usize",
        details: e.to_string(),
      })?;
    if bytes.remaining() < payload_len {
      return Err(ParseError::NotEnoughBytes {
        context: "FetchObject::deserialize(payload)",
        needed: payload_len,
        available: bytes.remaining(),
      });
    }
    let payload = bytes.copy_to_bytes(payload_len);

    let forwarding_preference = if is_datagram {
      ObjectForwardingPreference::Datagram
    } else {
      ObjectForwardingPreference::Subgroup
    };

    // For Datagram forwarding, keep subgroup_id in sync with object_id so the
    // relay's unified Object view matches other ingress paths.
    let subgroup_id = if is_datagram { object_id } else { subgroup_id };

    Ok(FetchObject::Object(FetchObjectPayload {
      group_id,
      subgroup_id,
      object_id,
      publisher_priority,
      forwarding_preference,
      properties,
      payload,
    }))
  }
}

fn serialize_payload(
  p: &FetchObjectPayload,
  prev: Option<&FetchObjectContext>,
  group_order: GroupOrder,
) -> Result<Bytes, ParseError> {
  let is_datagram = matches!(
    p.forwarding_preference,
    ObjectForwardingPreference::Datagram
  );

  // Group ID Delta field. Present for the first Object (delta = absolute Group ID)
  // and whenever the Group ID changes; absent when the Group ID is inherited.
  let group_delta: Option<u64> = match prev {
    None => Some(p.group_id),
    Some(pc) if pc.group_id == p.group_id => None,
    Some(pc) => {
      let delta = match group_order {
        GroupOrder::Descending => pc
          .group_id
          .checked_sub(p.group_id)
          .and_then(|d| d.checked_sub(1)),
        GroupOrder::Ascending | GroupOrder::Original => p
          .group_id
          .checked_sub(pc.group_id)
          .and_then(|d| d.checked_sub(1)),
      };
      Some(delta.ok_or(ParseError::ProtocolViolation {
        context: "FetchObject::serialize",
        details: format!(
          "group id not monotonic for {group_order:?}: prior={} current={}",
          pc.group_id, p.group_id
        ),
      })?)
    }
  };

  // Object ID Delta field (draft-18 §11.4.4.1):
  // - first Object, or a new group: absolute Object ID.
  // - same group, prior + 1: field omitted.
  // - same group otherwise: prior + delta.
  let object_delta: Option<u64> = match prev {
    None => Some(p.object_id),
    Some(_) if group_delta.is_some() => Some(p.object_id),
    Some(pc) if pc.object_id + 1 == p.object_id => None,
    Some(pc) => Some(p.object_id.checked_sub(pc.object_id).ok_or({
      ParseError::ProtocolViolation {
        context: "FetchObject::serialize",
        details: format!(
          "object id not monotonic: prior={} current={}",
          pc.object_id, p.object_id
        ),
      }
    })?),
  };

  let has_priority = match prev {
    Some(pc) => pc.publisher_priority != p.publisher_priority,
    None => true,
  };

  let has_properties = matches!(&p.properties, Some(v) if !v.is_empty());

  // Subgroup mode. The first Object (no prior) cannot reference a prior subgroup.
  let subgroup_mode = if is_datagram || p.subgroup_id == 0 {
    SUBGROUP_MODE_ZERO
  } else {
    match prev {
      Some(pc) if pc.subgroup_id == p.subgroup_id => SUBGROUP_MODE_PRIOR,
      Some(pc) if pc.subgroup_id + 1 == p.subgroup_id => SUBGROUP_MODE_PRIOR_PLUS_ONE,
      _ => SUBGROUP_MODE_PRESENT,
    }
  };

  let mut flags: u8 = subgroup_mode & FLAG_SUBGROUP_MODE_MASK;
  if object_delta.is_some() {
    flags |= FLAG_OBJECT_ID_PRESENT;
  }
  if group_delta.is_some() {
    flags |= FLAG_GROUP_ID_PRESENT;
  }
  if has_priority {
    flags |= FLAG_PRIORITY_PRESENT;
  }
  if has_properties {
    flags |= FLAG_PROPERTIES_PRESENT;
  }
  if is_datagram {
    flags |= FLAG_DATAGRAM;
  }

  let mut buf = BytesMut::new();
  buf.put_vi(flags as u64)?;

  if let Some(delta) = group_delta {
    buf.put_vi(delta)?;
  }
  if !is_datagram && subgroup_mode == SUBGROUP_MODE_PRESENT {
    buf.put_vi(p.subgroup_id)?;
  }
  if let Some(delta) = object_delta {
    buf.put_vi(delta)?;
  }
  if has_priority {
    buf.put_u8(p.publisher_priority);
  }
  if has_properties {
    let ext_buf = serialize_object_properties(p.properties.as_ref().unwrap())?;
    buf.put_vi(ext_buf.len() as u64)?;
    buf.extend_from_slice(&ext_buf);
  }

  buf.put_vi(p.payload.len() as u64)?;
  buf.extend_from_slice(&p.payload);

  Ok(buf.freeze())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::pair::KeyValuePair;

  fn sample_payload() -> FetchObjectPayload {
    FetchObjectPayload {
      group_id: 9,
      subgroup_id: 144,
      object_id: 10,
      publisher_priority: 255,
      forwarding_preference: ObjectForwardingPreference::Subgroup,
      properties: Some(vec![
        ObjectProperty::Unknown {
          kvp: KeyValuePair::try_new_varint(0, 10).unwrap(),
        },
        ObjectProperty::Unknown {
          kvp: KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
        },
      ]),
      payload: Bytes::from_static(b"01239gjawkk92837aldmi"),
    }
  }

  const ASC: GroupOrder = GroupOrder::Ascending;

  #[test]
  fn roundtrip_first_object() {
    let obj = FetchObject::Object(sample_payload());
    let mut buf = obj.serialize(None, ASC).unwrap();
    let parsed = FetchObject::deserialize(&mut buf, None, ASC).unwrap();
    assert_eq!(parsed, obj);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn roundtrip_inherited_fields() {
    let first = sample_payload();
    let obj1 = FetchObject::Object(first.clone());
    let mut second = first.clone();
    // Inheritable: same group, object_id = prior + 1, same subgroup, same priority, no properties.
    second.object_id = first.object_id + 1;
    second.properties = None;
    second.payload = Bytes::from_static(b"second");
    let obj2 = FetchObject::Object(second);

    let mut wire = BytesMut::new();
    wire.extend_from_slice(&obj1.serialize(None, ASC).unwrap());
    let ctx = obj1.context();
    wire.extend_from_slice(&obj2.serialize(ctx.as_ref(), ASC).unwrap());
    let mut wire = wire.freeze();

    let parsed1 = FetchObject::deserialize(&mut wire, None, ASC).unwrap();
    assert_eq!(parsed1, obj1);
    let ctx1 = parsed1.context();
    let parsed2 = FetchObject::deserialize(&mut wire, ctx1.as_ref(), ASC).unwrap();
    assert_eq!(parsed2, obj2);
    assert!(!wire.has_remaining());
  }

  /// A run of objects that cross a group boundary must round-trip, with the
  /// Object ID resetting inside the new group and the Group ID recovered from
  /// its delta.
  #[test]
  fn roundtrip_across_group_boundary() {
    let mut a = sample_payload();
    a.group_id = 4;
    a.object_id = 7;
    let mut b = a.clone();
    b.object_id = 8; // same group, prior + 1
    b.properties = None;
    let mut c = a.clone();
    c.group_id = 6; // new group two ahead (ascending delta = 1)
    c.object_id = 0; // object id resets
    c.properties = None;

    let objs = [
      FetchObject::Object(a),
      FetchObject::Object(b),
      FetchObject::Object(c),
    ];

    for order in [
      GroupOrder::Ascending,
      GroupOrder::Descending,
      GroupOrder::Original,
    ] {
      // Descending needs decreasing group ids; flip the run for that order.
      let run: Vec<FetchObject> = if order == GroupOrder::Descending {
        let mut a = sample_payload();
        a.group_id = 6;
        a.object_id = 7;
        let mut b = a.clone();
        b.object_id = 8;
        b.properties = None;
        let mut c = a.clone();
        c.group_id = 4;
        c.object_id = 0;
        c.properties = None;
        vec![
          FetchObject::Object(a),
          FetchObject::Object(b),
          FetchObject::Object(c),
        ]
      } else {
        objs.to_vec()
      };

      let mut wire = BytesMut::new();
      let mut ctx: Option<FetchObjectContext> = None;
      for o in &run {
        wire.extend_from_slice(&o.serialize(ctx.as_ref(), order).unwrap());
        ctx = o.context();
      }
      let mut wire = wire.freeze();

      let mut ctx: Option<FetchObjectContext> = None;
      for o in &run {
        let parsed = FetchObject::deserialize(&mut wire, ctx.as_ref(), order).unwrap();
        assert_eq!(&parsed, o, "order={order:?}");
        ctx = parsed.context();
      }
      assert!(!wire.has_remaining(), "order={order:?}");
    }
  }

  /// An Object ID delta that pushes the value past 2^64-1 is a PROTOCOL_VIOLATION.
  #[test]
  fn object_id_delta_wrap_closes_session() {
    let prev = FetchObjectContext {
      group_id: 3,
      subgroup_id: 0,
      object_id: u64::MAX - 1,
      publisher_priority: 5,
    };
    // Same group (group delta absent), object delta present = 5 → prior + 5 wraps.
    let flags = SUBGROUP_MODE_ZERO | FLAG_OBJECT_ID_PRESENT;
    let mut buf = BytesMut::new();
    buf.put_vi(flags as u64).unwrap();
    buf.put_vi(5u64).unwrap();
    buf.put_vi(0u64).unwrap(); // payload length
    let mut frozen = buf.freeze();
    let err = FetchObject::deserialize(&mut frozen, Some(&prev), ASC).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  /// An ascending Group ID delta that overflows 2^64-1 is a PROTOCOL_VIOLATION.
  #[test]
  fn group_id_delta_wrap_closes_session() {
    let prev = FetchObjectContext {
      group_id: u64::MAX - 1,
      subgroup_id: 0,
      object_id: 0,
      publisher_priority: 5,
    };
    // Group delta present = 5 → prior + 5 + 1 wraps; object delta present (absolute).
    let flags = SUBGROUP_MODE_ZERO | FLAG_GROUP_ID_PRESENT | FLAG_OBJECT_ID_PRESENT;
    let mut buf = BytesMut::new();
    buf.put_vi(flags as u64).unwrap();
    buf.put_vi(5u64).unwrap(); // group delta
    buf.put_vi(0u64).unwrap(); // object id (absolute)
    buf.put_vi(0u64).unwrap(); // payload length
    let mut frozen = buf.freeze();
    let err = FetchObject::deserialize(&mut frozen, Some(&prev), ASC).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  /// A descending Group ID delta that underflows below 0 is a PROTOCOL_VIOLATION.
  #[test]
  fn group_id_delta_underflow_closes_session() {
    let prev = FetchObjectContext {
      group_id: 0,
      subgroup_id: 0,
      object_id: 0,
      publisher_priority: 5,
    };
    let flags = SUBGROUP_MODE_ZERO | FLAG_GROUP_ID_PRESENT | FLAG_OBJECT_ID_PRESENT;
    let mut buf = BytesMut::new();
    buf.put_vi(flags as u64).unwrap();
    buf.put_vi(0u64).unwrap(); // group delta → prior - (0 + 1) underflows
    buf.put_vi(0u64).unwrap();
    buf.put_vi(0u64).unwrap();
    let mut frozen = buf.freeze();
    let err =
      FetchObject::deserialize(&mut frozen, Some(&prev), GroupOrder::Descending).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn roundtrip_datagram_preference() {
    let mut p = sample_payload();
    p.forwarding_preference = ObjectForwardingPreference::Datagram;
    // For datagrams, subgroup_id is synthesized to match object_id on decode.
    p.subgroup_id = p.object_id;
    let obj = FetchObject::Object(p);
    let mut buf = obj.serialize(None, ASC).unwrap();
    let parsed = FetchObject::deserialize(&mut buf, None, ASC).unwrap();
    assert_eq!(parsed, obj);
  }

  #[test]
  fn roundtrip_end_of_non_existent_range() {
    let obj = FetchObject::EndOfRange {
      kind: EndOfRangeKind::NonExistent,
      group_id: 7,
      object_id: 42,
    };
    let mut buf = obj.serialize(None, ASC).unwrap();
    let parsed = FetchObject::deserialize(&mut buf, None, ASC).unwrap();
    assert_eq!(parsed, obj);
  }

  #[test]
  fn roundtrip_end_of_unknown_range() {
    let obj = FetchObject::EndOfRange {
      kind: EndOfRangeKind::Unknown,
      group_id: 100,
      object_id: 0,
    };
    let mut buf = obj.serialize(None, ASC).unwrap();
    let parsed = FetchObject::deserialize(&mut buf, None, ASC).unwrap();
    assert_eq!(parsed, obj);
  }

  #[test]
  fn reject_reserved_high_flag_value() {
    // 0x80 is ≥ 128 but not 0x8C / 0x10C → protocol violation.
    let mut buf = BytesMut::new();
    buf.put_vi(0x80u64).unwrap();
    buf.put_vi(1u64).unwrap();
    buf.put_vi(1u64).unwrap();
    let mut frozen = buf.freeze();
    let err = FetchObject::deserialize(&mut frozen, None, ASC).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn reject_first_object_inheriting_priority() {
    // flags = subgroup_mode present | object_id present | group_id present, priority absent → violation.
    let flags = SUBGROUP_MODE_PRESENT | FLAG_OBJECT_ID_PRESENT | FLAG_GROUP_ID_PRESENT;
    let mut buf = BytesMut::new();
    buf.put_vi(flags as u64).unwrap();
    buf.put_vi(1u64).unwrap();
    buf.put_vi(1u64).unwrap();
    buf.put_vi(1u64).unwrap();
    buf.put_vi(0u64).unwrap();
    let mut frozen = buf.freeze();
    let err = FetchObject::deserialize(&mut frozen, None, ASC).unwrap_err();
    assert!(matches!(err, ParseError::ProtocolViolation { .. }));
  }

  #[test]
  fn excess_bytes_preserved() {
    let obj = FetchObject::Object(sample_payload());
    let serialized = obj.serialize(None, ASC).unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();
    let parsed = FetchObject::deserialize(&mut buf, None, ASC).unwrap();
    assert_eq!(parsed, obj);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn partial_message_fails() {
    let obj = FetchObject::Object(sample_payload());
    let buf = obj.serialize(None, ASC).unwrap();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let result = FetchObject::deserialize(&mut partial, None, ASC);
    assert!(result.is_err());
  }
}
