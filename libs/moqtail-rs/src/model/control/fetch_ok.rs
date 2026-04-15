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

use super::constant::ControlMessageType;
use super::control_message::ControlMessageTrait;
use crate::model::common::location::Location;
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use crate::model::extension_header::track_extension::{
  TrackExtension, deserialize_track_extensions, serialize_track_extensions,
};
use crate::model::parameter::message_parameter::{
  MessageParameter, deserialize_message_parameters,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct FetchOk {
  pub request_id: u64,
  pub end_of_track: bool,
  pub end_location: Location,
  pub subscribe_parameters: Vec<MessageParameter>,
  pub track_extensions: Vec<TrackExtension>,
}

impl FetchOk {
  pub fn new(
    request_id: u64,
    end_of_track: bool,
    end_location: Location,
    subscribe_parameters: Vec<MessageParameter>,
    track_extensions: Vec<TrackExtension>,
  ) -> Self {
    Self {
      request_id,
      end_of_track,
      end_location,
      subscribe_parameters,
      track_extensions,
    }
  }
}

impl ControlMessageTrait for FetchOk {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::FetchOk)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.put_u8(if self.end_of_track { 1u8 } else { 0u8 });
    payload.extend_from_slice(&self.end_location.serialize()?);
    payload.put_vi(self.subscribe_parameters.len())?;
    for param in &self.subscribe_parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    // Track Extensions (no length prefix; bounded by outer message Length field)
    payload.extend_from_slice(&serialize_track_extensions(&self.track_extensions)?);

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "FetchOk::serialize",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;
    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);
    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let request_id = payload.get_vi()?;

    if payload.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "FetchOk::parse_payload(end_of_track)",
        needed: 1,
        available: 0,
      });
    }
    let end_of_track_raw = payload.get_u8();
    let end_of_track = match end_of_track_raw {
      0 => false,
      1 => true,
      _ => {
        return Err(ParseError::ProtocolViolation {
          context: "FetchOk::parse_payload(end_of_track)",
          details: format!("Invalid value for end of track {end_of_track_raw}"),
        });
      }
    };

    let end_location = Location::deserialize(payload)?;

    let param_count = payload.get_vi()?;
    let subscribe_parameters =
      deserialize_message_parameters(payload, param_count, ControlMessageType::FetchOk)?;

    // Track Extensions: consume whatever remains in the payload
    let track_extensions = deserialize_track_extensions(payload)?;

    Ok(Box::new(FetchOk {
      request_id,
      end_of_track,
      end_location,
      subscribe_parameters,
      track_extensions,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::FetchOk
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use crate::model::control::constant::GroupOrder;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let fetch_ok = FetchOk {
      request_id: 271828,
      end_of_track: true,
      end_location: Location {
        group: 17,
        object: 57,
      },
      subscribe_parameters: vec![],
      track_extensions: vec![],
    };
    let mut buf = fetch_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::FetchOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = FetchOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, fetch_ok);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_with_group_order_param() {
    let fetch_ok = FetchOk {
      request_id: 271828,
      end_of_track: true,
      end_location: Location {
        group: 17,
        object: 57,
      },
      subscribe_parameters: vec![MessageParameter::new_group_order(GroupOrder::Ascending)],
      track_extensions: vec![],
    };
    let mut buf = fetch_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::FetchOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = FetchOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, fetch_ok);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_roundtrip_with_track_extensions() {
    let fetch_ok = FetchOk {
      request_id: 12345,
      end_of_track: false,
      end_location: Location {
        group: 5,
        object: 0,
      },
      subscribe_parameters: vec![],
      track_extensions: vec![
        TrackExtension::MaxCacheDuration { duration_ms: 60000 },
        TrackExtension::DefaultPublisherGroupOrder {
          order: GroupOrder::Ascending,
        },
      ],
    };
    let mut buf = fetch_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::FetchOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = FetchOk::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, fetch_ok);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let fetch_ok = FetchOk {
      request_id: 271828,
      end_of_track: true,
      end_location: Location {
        group: 17,
        object: 57,
      },
      subscribe_parameters: vec![],
      track_extensions: vec![],
    };

    let serialized = fetch_ok.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::FetchOk as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let mut payload = buf.copy_to_bytes(msg_length as usize);
    let deserialized = FetchOk::parse_payload(&mut payload).unwrap();
    assert_eq!(*deserialized, fetch_ok);
    assert!(!payload.has_remaining());
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let fetch_ok = FetchOk {
      request_id: 271828,
      end_of_track: true,
      end_location: Location {
        group: 17,
        object: 57,
      },
      subscribe_parameters: vec![],
      track_extensions: vec![],
    };
    let mut buf = fetch_ok.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::FetchOk as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = FetchOk::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }
}
