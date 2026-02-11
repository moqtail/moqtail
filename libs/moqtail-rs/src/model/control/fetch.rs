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

use super::constant::{ControlMessageType, FetchType, GroupOrder};
use super::control_message::ControlMessageTrait;
use crate::model::common::location::Location;
use crate::model::common::pair::KeyValuePair;
use crate::model::common::tuple::{Tuple, TupleField};
use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, PartialEq, Clone)]
pub struct StandAloneFetchProps {
  pub track_namespace: Tuple,
  pub track_name: TupleField,
  pub start_location: Location,
  pub end_location: Location,
}

#[derive(Debug, PartialEq, Clone)]
pub struct JoiningFetchProps {
  pub joining_request_id: u64,
  pub joining_start: u64,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Fetch {
  pub request_id: u64,
  pub subscriber_priority: u8,
  pub group_order: GroupOrder,
  pub fetch_type: FetchType,
  pub standalone_fetch_props: Option<StandAloneFetchProps>,
  pub joining_fetch_props: Option<JoiningFetchProps>,
  pub parameters: Vec<KeyValuePair>,
}

impl Fetch {
  /// Create a new standalone fetch request
  pub fn new_standalone(
    request_id: u64,
    subscriber_priority: u8,
    group_order: GroupOrder,
    standalone_fetch_props: StandAloneFetchProps,
    parameters: Vec<KeyValuePair>,
  ) -> Self {
    Self {
      request_id,
      subscriber_priority,
      group_order,
      fetch_type: FetchType::StandAlone,
      standalone_fetch_props: Some(standalone_fetch_props),
      joining_fetch_props: None,
      parameters,
    }
  }

  /// Create a new joining fetch request (absolute or relative)
  pub fn new_joining(
    request_id: u64,
    subscriber_priority: u8,
    group_order: GroupOrder,
    fetch_type: FetchType,
    joining_request_id: u64,
    joining_start: u64,
    parameters: Vec<KeyValuePair>,
  ) -> Result<Self, &'static str> {
    match fetch_type {
      FetchType::AbsoluteFetch | FetchType::RelativeFetch => Ok(Self {
        request_id,
        subscriber_priority,
        group_order,
        fetch_type,
        standalone_fetch_props: None,
        joining_fetch_props: Some(JoiningFetchProps {
          joining_request_id,
          joining_start,
        }),
        parameters,
      }),
      FetchType::StandAlone => Err("Use new_standalone for standalone fetch requests"),
    }
  }

  /// Create a new fetch request (legacy method - prefer new_standalone or new_joining)
  #[deprecated(note = "Use new_standalone or new_joining for better ergonomics")]
  pub fn new(
    request_id: u64,
    subscriber_priority: u8,
    group_order: GroupOrder,
    fetch_type: FetchType,
    standalone_fetch_props: Option<StandAloneFetchProps>,
    joining_fetch_props: Option<JoiningFetchProps>,
    parameters: Vec<KeyValuePair>,
  ) -> Self {
    Self {
      request_id,
      subscriber_priority,
      group_order,
      fetch_type,
      standalone_fetch_props,
      joining_fetch_props,
      parameters,
    }
  }
}

impl ControlMessageTrait for Fetch {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::Fetch as u64)?;

    let mut payload = BytesMut::new();
    payload.put_vi(self.request_id)?;
    payload.put_u8(self.subscriber_priority);
    payload.put_u8(self.group_order as u8);
    payload.put_vi(self.fetch_type)?;
    match &self.fetch_type {
      FetchType::AbsoluteFetch => {
        let props = self.joining_fetch_props.as_ref().unwrap();
        payload.put_vi(props.joining_request_id)?;
        payload.put_vi(props.joining_start)?;
      }
      FetchType::RelativeFetch => {
        let props = self.joining_fetch_props.as_ref().unwrap();
        payload.put_vi(props.joining_request_id)?;
        payload.put_vi(props.joining_start)?;
      }
      FetchType::StandAlone => {
        let props = self.standalone_fetch_props.as_ref().unwrap();
        payload.extend_from_slice(&props.track_namespace.serialize()?);
        payload.put_vi(props.track_name.len())?;
        payload.extend_from_slice(props.track_name.as_bytes());
        payload.extend_from_slice(&props.start_location.serialize()?);
        payload.extend_from_slice(&props.end_location.serialize()?);
      }
    }

    payload.put_vi(self.parameters.len())?;
    for param in &self.parameters {
      payload.extend_from_slice(&param.serialize()?);
    }

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "Fetch::serialize(payload_length)",
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
        context: "Fetch::parse_payload(subscriber_priority)",
        needed: 1,
        available: 0,
      });
    }
    let subscriber_priority = payload.get_u8();

    if payload.remaining() < 1 {
      return Err(ParseError::NotEnoughBytes {
        context: "Fetch::parse_payload(group_order)",
        needed: 1,
        available: 0,
      });
    }
    let group_order_raw = payload.get_u8();
    let group_order = GroupOrder::try_from(group_order_raw)?;

    let fetch_type_raw = payload.get_vi()?;
    let fetch_type = FetchType::try_from(fetch_type_raw)?;

    let mut standalone_fetch_props: Option<StandAloneFetchProps> = None;
    let mut joining_fetch_props: Option<JoiningFetchProps> = None;

    match fetch_type {
      FetchType::AbsoluteFetch => {
        let joining_request_id = payload.get_vi()?;
        let joining_start = payload.get_vi()?;
        joining_fetch_props = Some(JoiningFetchProps {
          joining_request_id,
          joining_start,
        });
      }
      FetchType::RelativeFetch => {
        let joining_request_id = payload.get_vi()?;
        let joining_start = payload.get_vi()?;
        joining_fetch_props = Some(JoiningFetchProps {
          joining_request_id,
          joining_start,
        });
      }
      FetchType::StandAlone => {
        let track_namespace = Tuple::deserialize(payload)?;
        let track_name_len = payload.get_vi()? as usize;
        if payload.remaining() < track_name_len {
          return Err(ParseError::NotEnoughBytes {
            context: "Fetch::parse_payload(track_name)",
            needed: track_name_len,
            available: payload.remaining(),
          });
        }
        let track_name = TupleField::new(payload.copy_to_bytes(track_name_len));
        let start_location = Location::deserialize(payload)?;
        let end_location = Location::deserialize(payload)?;

        standalone_fetch_props = Some(StandAloneFetchProps {
          track_namespace,
          track_name,
          start_location,
          end_location,
        });
      }
    }

    let param_count_u64 = payload.get_vi()?;
    let param_count: usize =
      param_count_u64
        .try_into()
        .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
          context: "Fetch::deserialize(param_count)",
          from_type: "u64",
          to_type: "usize",
          details: e.to_string(),
        })?;

    let mut parameters = Vec::with_capacity(param_count);
    for _ in 0..param_count {
      let param = KeyValuePair::deserialize(payload)?;
      parameters.push(param);
    }

    Ok(Box::new(Fetch {
      request_id,
      subscriber_priority,
      group_order,
      fetch_type,
      standalone_fetch_props,
      joining_fetch_props,
      parameters,
    }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::Fetch
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use crate::model::common::tuple::TupleField;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let request_id = 161803;
    let subscriber_priority = 15u8;
    let group_order = GroupOrder::Descending;
    let fetch_type = FetchType::AbsoluteFetch;
    let joining_fetch_props = JoiningFetchProps {
      joining_request_id: 119,
      joining_start: 73,
    };
    let parameters = vec![
      KeyValuePair::try_new_varint(4444, 12321).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"fetch me ok")).unwrap(),
    ];
    let fetch = Fetch {
      request_id,
      subscriber_priority,
      group_order,
      fetch_type,
      standalone_fetch_props: None,
      joining_fetch_props: Some(joining_fetch_props),
      parameters,
    };

    let mut buf = fetch.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Fetch as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());
    let deserialized = Fetch::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, fetch);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_excess_roundtrip() {
    let request_id = 161803;
    let subscriber_priority = 15u8;
    let group_order = GroupOrder::Descending;
    let fetch_type = FetchType::AbsoluteFetch;
    let joining_fetch_props = JoiningFetchProps {
      joining_request_id: 119,
      joining_start: 73,
    };
    let parameters = vec![
      KeyValuePair::try_new_varint(4444, 12321).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"fetch me ok")).unwrap(),
    ];
    let fetch = Fetch {
      request_id,
      subscriber_priority,
      group_order,
      fetch_type,
      standalone_fetch_props: None,
      joining_fetch_props: Some(joining_fetch_props),
      parameters,
    };

    let serialized = fetch.serialize().unwrap();
    let mut excess = BytesMut::new();
    excess.extend_from_slice(&serialized);
    excess.extend_from_slice(&[9u8, 1u8, 1u8]);
    let mut buf = excess.freeze();

    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Fetch as u64);
    let msg_length = buf.get_u16();

    assert_eq!(msg_length as usize, buf.remaining() - 3);
    let deserialized = Fetch::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, fetch);
    assert_eq!(buf.chunk(), &[9u8, 1u8, 1u8]);
  }

  #[test]
  fn test_partial_message() {
    let request_id = 161803;
    let subscriber_priority = 15u8;
    let group_order = GroupOrder::Descending;
    let fetch_type = FetchType::AbsoluteFetch;
    let joining_fetch_props = JoiningFetchProps {
      joining_request_id: 119,
      joining_start: 73,
    };
    let parameters = vec![
      KeyValuePair::try_new_varint(4444, 12321).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"fetch me ok")).unwrap(),
    ];
    let fetch = Fetch {
      request_id,
      subscriber_priority,
      group_order,
      fetch_type,
      standalone_fetch_props: None,
      joining_fetch_props: Some(joining_fetch_props),
      parameters,
    };

    let mut buf = fetch.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Fetch as u64);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let deserialized = Fetch::parse_payload(&mut partial);
    assert!(deserialized.is_err());
  }

  #[test]
  fn test_new_standalone_constructor() {
    let mut track_namespace = Tuple::new();
    track_namespace.add(TupleField::from_utf8("test"));
    track_namespace.add(TupleField::from_utf8("namespace"));
    let track_name = TupleField::from_utf8("video_track");
    let start_location = Location::new(5, 10);
    let end_location = Location::new(15, 20);
    let parameters = vec![KeyValuePair::try_new_varint(100, 200).unwrap()];
    let standalone_fetch_props = StandAloneFetchProps {
      track_namespace,
      track_name,
      start_location,
      end_location,
    };

    let fetch = Fetch::new_standalone(
      42,
      1,
      GroupOrder::Ascending,
      standalone_fetch_props.clone(),
      parameters.clone(),
    );

    assert_eq!(fetch.request_id, 42);
    assert_eq!(fetch.subscriber_priority, 1);
    assert_eq!(fetch.group_order, GroupOrder::Ascending);
    assert_eq!(fetch.fetch_type, FetchType::StandAlone);
    assert!(fetch.standalone_fetch_props.is_some());
    assert!(fetch.joining_fetch_props.is_none());

    let props = fetch.standalone_fetch_props.unwrap();
    assert_eq!(
      props.track_namespace,
      standalone_fetch_props.track_namespace
    );
    assert_eq!(props.track_name, standalone_fetch_props.track_name);
    assert_eq!(props.start_location, standalone_fetch_props.start_location);
    assert_eq!(props.end_location, standalone_fetch_props.end_location);
    assert_eq!(fetch.parameters, parameters);
  }

  #[test]
  fn test_new_joining_constructor_absolute() {
    let parameters = vec![KeyValuePair::try_new_varint(300, 400).unwrap()];

    let fetch = Fetch::new_joining(
      123,
      5,
      GroupOrder::Descending,
      FetchType::AbsoluteFetch,
      456,
      789,
      parameters.clone(),
    )
    .unwrap();

    assert_eq!(fetch.request_id, 123);
    assert_eq!(fetch.subscriber_priority, 5);
    assert_eq!(fetch.group_order, GroupOrder::Descending);
    assert_eq!(fetch.fetch_type, FetchType::AbsoluteFetch);
    assert!(fetch.standalone_fetch_props.is_none());
    assert!(fetch.joining_fetch_props.is_some());

    let props = fetch.joining_fetch_props.unwrap();
    assert_eq!(props.joining_request_id, 456);
    assert_eq!(props.joining_start, 789);
    assert_eq!(fetch.parameters, parameters);
  }

  #[test]
  fn test_new_joining_constructor_relative() {
    let parameters = vec![];

    let fetch = Fetch::new_joining(
      111,
      2,
      GroupOrder::Ascending,
      FetchType::RelativeFetch,
      222,
      333,
      parameters.clone(),
    )
    .unwrap();

    assert_eq!(fetch.fetch_type, FetchType::RelativeFetch);
    let props = fetch.joining_fetch_props.unwrap();
    assert_eq!(props.joining_request_id, 222);
    assert_eq!(props.joining_start, 333);
  }

  #[test]
  fn test_new_joining_constructor_rejects_standalone() {
    let parameters = vec![];

    let result = Fetch::new_joining(
      111,
      2,
      GroupOrder::Ascending,
      FetchType::StandAlone,
      222,
      333,
      parameters,
    );

    assert!(result.is_err());
    assert_eq!(
      result.unwrap_err(),
      "Use new_standalone for standalone fetch requests"
    );
  }
}
