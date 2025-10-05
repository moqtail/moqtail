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

use crate::model::common::varint::BufVarIntExt;
use crate::model::error::ParseError;
use bytes::{Buf, Bytes};

use super::{
  client_setup::ClientSetup, constant::ControlMessageType, fetch::Fetch, fetch_cancel::FetchCancel,
  fetch_error::FetchError, fetch_ok::FetchOk, goaway::GoAway, max_request_id::MaxRequestId,
  publish::Publish, publish_done::PublishDone, publish_error::PublishError,
  publish_namespace::PublishNamespace, publish_namespace_cancel::PublishNamespaceCancel,
  publish_namespace_done::PublishNamespaceDone, publish_namespace_error::PublishNamespaceError,
  publish_namespace_ok::PublishNamespaceOk, publish_ok::PublishOk,
  requests_blocked::RequestsBlocked, server_setup::ServerSetup, subscribe::Subscribe,
  subscribe_error::SubscribeError, subscribe_namespace::SubscribeNamespace,
  subscribe_namespace_error::SubscribeNamespaceError, subscribe_namespace_ok::SubscribeNamespaceOk,
  subscribe_ok::SubscribeOk, subscribe_update::SubscribeUpdate, track_status::TrackStatus,
  track_status_error::TrackStatusError, track_status_ok::TrackStatusOk, unsubscribe::Unsubscribe,
  unsubscribe_namespace::UnsubscribeNamespace,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ControlMessage {
  PublishNamespace(Box<PublishNamespace>),
  PublishNamespaceCancel(Box<PublishNamespaceCancel>),
  PublishNamespaceError(Box<PublishNamespaceError>),
  PublishNamespaceOk(Box<PublishNamespaceOk>),
  Publish(Box<Publish>),
  PublishOk(Box<PublishOk>),
  PublishError(Box<PublishError>),
  PublishDone(Box<PublishDone>),
  ClientSetup(Box<ClientSetup>),
  Fetch(Box<Fetch>),
  FetchCancel(Box<FetchCancel>),
  FetchError(Box<FetchError>),
  FetchOk(Box<FetchOk>),
  Goaway(Box<GoAway>),
  MaxRequestId(Box<MaxRequestId>),
  ServerSetup(Box<ServerSetup>),
  Subscribe(Box<Subscribe>),
  SubscribeError(Box<SubscribeError>),
  SubscribeOk(Box<SubscribeOk>),
  SubscribeUpdate(Box<SubscribeUpdate>),
  RequestsBlocked(Box<RequestsBlocked>),
  TrackStatus(Box<TrackStatus>),
  TrackStatusOk(Box<TrackStatusOk>),
  TrackStatusError(Box<TrackStatusError>),
  PublishNamespaceDone(Box<PublishNamespaceDone>),
  Unsubscribe(Box<Unsubscribe>),
  SubscribeNamespace(Box<SubscribeNamespace>),
  SubscribeNamespaceOk(Box<SubscribeNamespaceOk>),
  SubscribeNamespaceError(Box<SubscribeNamespaceError>),
  UnsubscribeNamespace(Box<UnsubscribeNamespace>),
}

pub trait ControlMessageTrait: std::fmt::Debug {
  fn serialize(&self) -> Result<Bytes, ParseError>;
  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError>
  where
    Self: Sized;
  fn get_type(&self) -> ControlMessageType;
}

impl ControlMessage {
  pub fn deserialize(bytes: &mut Bytes) -> Result<Self, ParseError> {
    let message_type = bytes.get_vi()?;
    let msg_type = ControlMessageType::try_from(message_type)?;

    if bytes.remaining() < 2 {
      return Err(ParseError::NotEnoughBytes {
        context: "ControlMessage::deserialize(payload_length)",
        needed: 2,
        available: 0,
      });
    }
    let payload_length = bytes.get_u16() as usize;

    if bytes.remaining() < payload_length {
      return Err(ParseError::NotEnoughBytes {
        context: "ControlMessage::deserialize(payload_length)",
        needed: payload_length,
        available: bytes.remaining(),
      });
    }

    let mut payload = bytes.copy_to_bytes(payload_length);
    let message = match msg_type {
      ControlMessageType::PublishNamespace => {
        PublishNamespace::parse_payload(&mut payload).map(ControlMessage::PublishNamespace)
      }
      ControlMessageType::PublishNamespaceCancel => {
        PublishNamespaceCancel::parse_payload(&mut payload)
          .map(ControlMessage::PublishNamespaceCancel)
      }
      ControlMessageType::PublishNamespaceDone => {
        PublishNamespaceDone::parse_payload(&mut payload).map(ControlMessage::PublishNamespaceDone)
      }
      ControlMessageType::PublishNamespaceError => {
        PublishNamespaceError::parse_payload(&mut payload)
          .map(ControlMessage::PublishNamespaceError)
      }
      ControlMessageType::PublishNamespaceOk => {
        PublishNamespaceOk::parse_payload(&mut payload).map(ControlMessage::PublishNamespaceOk)
      }
      ControlMessageType::Publish => {
        Publish::parse_payload(&mut payload).map(ControlMessage::Publish)
      }
      ControlMessageType::PublishOk => {
        PublishOk::parse_payload(&mut payload).map(ControlMessage::PublishOk)
      }
      ControlMessageType::PublishError => {
        PublishError::parse_payload(&mut payload).map(ControlMessage::PublishError)
      }
      ControlMessageType::PublishDone => {
        PublishDone::parse_payload(&mut payload).map(ControlMessage::PublishDone)
      }
      ControlMessageType::ClientSetup => {
        ClientSetup::parse_payload(&mut payload).map(ControlMessage::ClientSetup)
      }
      ControlMessageType::Fetch => Fetch::parse_payload(&mut payload).map(ControlMessage::Fetch),
      ControlMessageType::FetchCancel => {
        FetchCancel::parse_payload(&mut payload).map(ControlMessage::FetchCancel)
      }
      ControlMessageType::FetchError => {
        FetchError::parse_payload(&mut payload).map(ControlMessage::FetchError)
      }
      ControlMessageType::FetchOk => {
        FetchOk::parse_payload(&mut payload).map(ControlMessage::FetchOk)
      }
      ControlMessageType::GoAway => GoAway::parse_payload(&mut payload).map(ControlMessage::Goaway),
      ControlMessageType::MaxRequestId => {
        MaxRequestId::parse_payload(&mut payload).map(ControlMessage::MaxRequestId)
      }
      ControlMessageType::ServerSetup => {
        ServerSetup::parse_payload(&mut payload).map(ControlMessage::ServerSetup)
      }
      ControlMessageType::Subscribe => {
        Subscribe::parse_payload(&mut payload).map(ControlMessage::Subscribe)
      }
      ControlMessageType::SubscribeError => {
        SubscribeError::parse_payload(&mut payload).map(ControlMessage::SubscribeError)
      }
      ControlMessageType::SubscribeOk => {
        SubscribeOk::parse_payload(&mut payload).map(ControlMessage::SubscribeOk)
      }
      ControlMessageType::SubscribeUpdate => {
        SubscribeUpdate::parse_payload(&mut payload).map(ControlMessage::SubscribeUpdate)
      }
      ControlMessageType::RequestsBlocked => {
        RequestsBlocked::parse_payload(&mut payload).map(ControlMessage::RequestsBlocked)
      }
      ControlMessageType::TrackStatus => {
        TrackStatus::parse_payload(&mut payload).map(ControlMessage::TrackStatus)
      }
      ControlMessageType::TrackStatusOk => {
        TrackStatusOk::parse_payload(&mut payload).map(ControlMessage::TrackStatusOk)
      }
      ControlMessageType::TrackStatusError => {
        TrackStatusError::parse_payload(&mut payload).map(ControlMessage::TrackStatusError)
      }
      ControlMessageType::Unsubscribe => {
        Unsubscribe::parse_payload(&mut payload).map(ControlMessage::Unsubscribe)
      }
      ControlMessageType::SubscribeNamespace => {
        SubscribeNamespace::parse_payload(&mut payload).map(ControlMessage::SubscribeNamespace)
      }
      ControlMessageType::SubscribeNamespaceOk => {
        SubscribeNamespaceOk::parse_payload(&mut payload).map(ControlMessage::SubscribeNamespaceOk)
      }
      ControlMessageType::SubscribeNamespaceError => {
        SubscribeNamespaceError::parse_payload(&mut payload)
          .map(ControlMessage::SubscribeNamespaceError)
      }
      ControlMessageType::UnsubscribeNamespace => {
        UnsubscribeNamespace::parse_payload(&mut payload).map(ControlMessage::UnsubscribeNamespace)
      }
    }
    .map_err(|err| ParseError::ProtocolViolation {
      context: "ControlMessage::deserialize(payload)",
      details: err.to_string(),
    })?;

    if payload.has_remaining() {
      return Err(ParseError::ProtocolViolation {
        context: "ControlMessage::deserialize(final_check)",
        details: format!(
          "Extra {} bytes remaining in payload after parsing",
          payload.remaining()
        ),
      });
    };
    Ok(message)
  }

  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    match self {
      ControlMessage::PublishNamespace(msg) => msg.serialize(),
      ControlMessage::PublishNamespaceCancel(msg) => msg.serialize(),
      ControlMessage::PublishNamespaceDone(msg) => msg.serialize(),
      ControlMessage::PublishNamespaceError(msg) => msg.serialize(),
      ControlMessage::PublishNamespaceOk(msg) => msg.serialize(),
      ControlMessage::Publish(msg) => msg.serialize(),
      ControlMessage::PublishOk(msg) => msg.serialize(),
      ControlMessage::PublishError(msg) => msg.serialize(),
      ControlMessage::PublishDone(msg) => msg.serialize(),
      ControlMessage::ClientSetup(msg) => msg.serialize(),
      ControlMessage::Fetch(msg) => msg.serialize(),
      ControlMessage::FetchCancel(msg) => msg.serialize(),
      ControlMessage::FetchError(msg) => msg.serialize(),
      ControlMessage::FetchOk(msg) => msg.serialize(),
      ControlMessage::Goaway(msg) => msg.serialize(),
      ControlMessage::MaxRequestId(msg) => msg.serialize(),
      ControlMessage::ServerSetup(msg) => msg.serialize(),
      ControlMessage::Subscribe(msg) => msg.serialize(),
      ControlMessage::SubscribeError(msg) => msg.serialize(),
      ControlMessage::SubscribeOk(msg) => msg.serialize(),
      ControlMessage::SubscribeUpdate(msg) => msg.serialize(),
      ControlMessage::RequestsBlocked(msg) => msg.serialize(),
      ControlMessage::TrackStatus(msg) => msg.serialize(),
      ControlMessage::TrackStatusOk(msg) => msg.serialize(),
      ControlMessage::TrackStatusError(msg) => msg.serialize(),
      ControlMessage::Unsubscribe(msg) => msg.serialize(),
      ControlMessage::SubscribeNamespace(msg) => msg.serialize(),
      ControlMessage::SubscribeNamespaceOk(msg) => msg.serialize(),
      ControlMessage::SubscribeNamespaceError(msg) => msg.serialize(),
      ControlMessage::UnsubscribeNamespace(msg) => msg.serialize(),
    }
  }

  /// Returns the message type of the control message.
  pub fn get_type(&self) -> ControlMessageType {
    match self {
      ControlMessage::PublishNamespace(_) => ControlMessageType::PublishNamespace,
      ControlMessage::PublishNamespaceCancel(_) => ControlMessageType::PublishNamespaceCancel,
      ControlMessage::PublishNamespaceDone(_) => ControlMessageType::PublishNamespaceDone,
      ControlMessage::PublishNamespaceError(_) => ControlMessageType::PublishNamespaceError,
      ControlMessage::PublishNamespaceOk(_) => ControlMessageType::PublishNamespaceOk,
      ControlMessage::Publish(_) => ControlMessageType::Publish,
      ControlMessage::PublishOk(_) => ControlMessageType::PublishOk,
      ControlMessage::PublishError(_) => ControlMessageType::PublishError,
      ControlMessage::PublishDone(_) => ControlMessageType::PublishDone,
      ControlMessage::ClientSetup(_) => ControlMessageType::ClientSetup,
      ControlMessage::Fetch(_) => ControlMessageType::Fetch,
      ControlMessage::FetchCancel(_) => ControlMessageType::FetchCancel,
      ControlMessage::FetchError(_) => ControlMessageType::FetchError,
      ControlMessage::FetchOk(_) => ControlMessageType::FetchOk,
      ControlMessage::Goaway(_) => ControlMessageType::GoAway,
      ControlMessage::MaxRequestId(_) => ControlMessageType::MaxRequestId,
      ControlMessage::ServerSetup(_) => ControlMessageType::ServerSetup,
      ControlMessage::Subscribe(_) => ControlMessageType::Subscribe,
      ControlMessage::SubscribeError(_) => ControlMessageType::SubscribeError,
      ControlMessage::SubscribeOk(_) => ControlMessageType::SubscribeOk,
      ControlMessage::SubscribeUpdate(_) => ControlMessageType::SubscribeUpdate,
      ControlMessage::RequestsBlocked(_) => ControlMessageType::RequestsBlocked,
      ControlMessage::TrackStatus(_) => ControlMessageType::TrackStatus,
      ControlMessage::TrackStatusOk(_) => ControlMessageType::TrackStatusOk,
      ControlMessage::TrackStatusError(_) => ControlMessageType::TrackStatusError,
      ControlMessage::Unsubscribe(_) => ControlMessageType::Unsubscribe,
      ControlMessage::SubscribeNamespace(_) => ControlMessageType::SubscribeNamespace,
      ControlMessage::SubscribeNamespaceOk(_) => ControlMessageType::SubscribeNamespaceOk,
      ControlMessage::SubscribeNamespaceError(_) => ControlMessageType::SubscribeNamespaceError,
      ControlMessage::UnsubscribeNamespace(_) => ControlMessageType::UnsubscribeNamespace,
    }
  }
}

#[cfg(test)]
mod tests {
  use crate::model::common::{pair::KeyValuePair, tuple::Tuple};

  use super::*;

  #[test]
  fn test_announce_roundtrip() {
    let request_id = 12345;
    let track_namespace = Tuple::from_utf8_path("god/dayyum");
    let parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ];
    let announce = PublishNamespace {
      request_id,
      track_namespace,
      parameters,
    };

    let mut buf = announce.serialize().unwrap();
    let deserialized = ControlMessage::deserialize(&mut buf).unwrap();
    if let ControlMessage::PublishNamespace(deserialized_announce) = deserialized {
      assert_eq!(*deserialized_announce, announce);
    } else {
      panic!("Expected ControlMessage::PublishNamespace variant");
    }
    assert!(!buf.has_remaining());
  }
}
