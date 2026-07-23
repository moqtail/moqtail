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

use super::constant::ControlMessageType;
use super::control_message::ControlMessageTrait;
use crate::model::common::pair::{
  KeyValuePair, deserialize_kvp_list_until_empty, serialize_kvp_list,
};
use crate::model::common::varint::BufMutVarIntExt;
use crate::model::error::{ParseError, TerminationCode};
use crate::model::parameter::constant::SetupOptionType;
use crate::transport::connection::TransportKind;
use bytes::{BufMut, Bytes, BytesMut};

/// The first message each endpoint sends on its control stream.
///
/// Both peers send the same message; there are no separate client and server forms and
/// no version fields, since version negotiation happens over ALPN.
///
/// Some options are client-only — see [`SetupOptionType`](crate::model::parameter::constant::SetupOptionType).
/// Whether a peer is allowed to send a given option depends on which side it is and on
/// the transport, neither of which is knowable here, so those rules are enforced by the
/// session rather than by parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
  pub setup_options: Vec<KeyValuePair>,
}

/// Which side sent a SETUP. Some options are client-only, so validating one depends on
/// knowing who sent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupSender {
  Client,
  Server,
}

impl Setup {
  pub fn new(setup_options: Vec<KeyValuePair>) -> Self {
    Setup { setup_options }
  }

  /// Checks the options a peer is allowed to send.
  ///
  /// AUTHORITY and PATH carry the authority and path of the MoQ URI for native QUIC,
  /// where there is no HTTP CONNECT to carry them. Both MUST NOT be sent by a server,
  /// and MUST NOT be used over WebTransport, which carries them itself. Either
  /// violation closes the session: `INVALID_AUTHORITY` for AUTHORITY, `INVALID_PATH`
  /// for PATH.
  ///
  /// Unknown options are ignored rather than rejected.
  pub fn validate_incoming(
    &self,
    sender: SetupSender,
    transport: TransportKind,
  ) -> Result<(), TerminationCode> {
    for kvp in &self.setup_options {
      let code = match SetupOptionType::try_from(kvp.get_type()) {
        Ok(SetupOptionType::Authority) => TerminationCode::InvalidAuthority,
        Ok(SetupOptionType::Path) => TerminationCode::InvalidPath,
        // Every other option may be sent by either peer on either transport.
        Ok(_) => continue,
        // receivers MUST ignore unrecognized Setup Options.
        Err(_) => continue,
      };

      if sender == SetupSender::Server || transport == TransportKind::WebTransport {
        return Err(code);
      }
    }
    Ok(())
  }
}

impl ControlMessageTrait for Setup {
  fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(ControlMessageType::Setup)?;

    // Setup Options span the whole payload, bounded by Length; there is no count.
    let payload = serialize_kvp_list(&self.setup_options)?;

    let payload_len: u16 = payload
      .len()
      .try_into()
      .map_err(|e: std::num::TryFromIntError| ParseError::CastingError {
        context: "Setup::serialize(payload_length)",
        from_type: "usize",
        to_type: "u16",
        details: e.to_string(),
      })?;

    buf.put_u16(payload_len);
    buf.extend_from_slice(&payload);

    Ok(buf.freeze())
  }

  fn parse_payload(payload: &mut Bytes) -> Result<Box<Self>, ParseError> {
    let setup_options = deserialize_kvp_list_until_empty(payload)?;
    Ok(Box::new(Setup { setup_options }))
  }

  fn get_type(&self) -> ControlMessageType {
    ControlMessageType::Setup
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::varint::BufVarIntExt;
  use bytes::Buf;

  #[test]
  fn test_roundtrip() {
    let setup_options = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"Set me up!")).unwrap(),
    ];
    let setup = Setup {
      setup_options: setup_options.clone(),
    };

    let mut buf = setup.serialize().unwrap();
    let msg_type = buf.get_vi().unwrap();
    assert_eq!(msg_type, ControlMessageType::Setup as u64);
    assert_eq!(msg_type, 0x2F00);
    let msg_length = buf.get_u16();
    assert_eq!(msg_length as usize, buf.remaining());

    let deserialized = Setup::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, setup);
    assert!(!buf.has_remaining());
  }

  #[test]
  fn test_grease_setup_option_is_preserved_and_ignored() {
    use crate::model::common::grease::grease_value;
    // grease_value(0) = 0x9D is odd (Bytes KVP).
    let grease =
      KeyValuePair::try_new_bytes(grease_value(0).unwrap(), Bytes::from_static(b"anything"))
        .unwrap();
    let setup = Setup {
      setup_options: vec![KeyValuePair::try_new_varint(0, 10).unwrap(), grease],
    };

    // The grease option survives a round-trip untouched.
    let mut buf = setup.serialize().unwrap();
    let _ = buf.get_vi().unwrap();
    let _ = buf.get_u16();
    let deserialized = Setup::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, setup);

    // And it is ignored by validation, even from a server over WebTransport,
    // where a real AUTHORITY/PATH option would be rejected.
    assert!(
      deserialized
        .validate_incoming(SetupSender::Server, TransportKind::WebTransport)
        .is_ok()
    );
  }

  #[test]
  fn test_roundtrip_no_options() {
    let setup = Setup {
      setup_options: vec![],
    };

    let mut buf = setup.serialize().unwrap();
    let _ = buf.get_vi().unwrap();
    let msg_length = buf.get_u16();
    assert_eq!(msg_length, 0, "no options means an empty payload");

    let deserialized = Setup::parse_payload(&mut buf).unwrap();
    assert_eq!(*deserialized, setup);
  }

  /// The options are bounded by Length, not preceded by a count.
  #[test]
  fn test_payload_carries_no_option_count() {
    let setup = Setup {
      setup_options: vec![KeyValuePair::try_new_varint(0, 10).unwrap()],
    };

    let mut buf = setup.serialize().unwrap();
    let _ = buf.get_vi().unwrap();
    let _ = buf.get_u16();

    // The payload must start with the first option's own bytes, not a count of 1.
    let expected = serialize_kvp_list(&setup.setup_options).unwrap();
    assert_eq!(&buf[..], &expected[..]);
  }

  fn setup_with(option: SetupOptionType) -> Setup {
    Setup {
      setup_options: vec![
        KeyValuePair::try_new_bytes(option as u64, Bytes::from_static(b"example.com")).unwrap(),
      ],
    }
  }

  #[test]
  fn authority_from_a_server_is_rejected() {
    assert_eq!(
      setup_with(SetupOptionType::Authority)
        .validate_incoming(SetupSender::Server, TransportKind::Quic)
        .unwrap_err(),
      TerminationCode::InvalidAuthority
    );
  }

  #[test]
  fn authority_over_webtransport_is_rejected() {
    assert_eq!(
      setup_with(SetupOptionType::Authority)
        .validate_incoming(SetupSender::Client, TransportKind::WebTransport)
        .unwrap_err(),
      TerminationCode::InvalidAuthority
    );
  }

  #[test]
  fn path_from_a_server_is_rejected() {
    assert_eq!(
      setup_with(SetupOptionType::Path)
        .validate_incoming(SetupSender::Server, TransportKind::Quic)
        .unwrap_err(),
      TerminationCode::InvalidPath
    );
  }

  #[test]
  fn path_over_webtransport_is_rejected() {
    assert_eq!(
      setup_with(SetupOptionType::Path)
        .validate_incoming(SetupSender::Client, TransportKind::WebTransport)
        .unwrap_err(),
      TerminationCode::InvalidPath
    );
  }

  #[test]
  fn client_only_options_are_accepted_from_a_client_over_quic() {
    for option in [SetupOptionType::Authority, SetupOptionType::Path] {
      assert!(
        setup_with(option)
          .validate_incoming(SetupSender::Client, TransportKind::Quic)
          .is_ok(),
        "{option:?} is what native QUIC uses these for"
      );
    }
  }

  #[test]
  fn other_options_are_accepted_from_either_peer_on_either_transport() {
    let setup = setup_with(SetupOptionType::MoqtImplementation);
    for sender in [SetupSender::Client, SetupSender::Server] {
      for transport in [TransportKind::Quic, TransportKind::WebTransport] {
        assert!(setup.validate_incoming(sender, transport).is_ok());
      }
    }
  }

  /// Receivers MUST ignore unrecognized Setup Options, so an unknown option is
  /// never a reason to close the session.
  #[test]
  fn unknown_options_are_ignored() {
    // 0x7A is unassigned in the Setup Options registry, and even, so it is a VarInt.
    let setup = Setup {
      setup_options: vec![KeyValuePair::try_new_varint(0x7A, 1).unwrap()],
    };
    assert!(
      setup
        .validate_incoming(SetupSender::Server, TransportKind::WebTransport)
        .is_ok()
    );
  }

  #[test]
  fn test_partial_message() {
    let setup = Setup {
      setup_options: vec![
        KeyValuePair::try_new_bytes(1, Bytes::from_static(b"truncated")).unwrap(),
      ],
    };
    let buf = setup.serialize().unwrap();
    let upper = buf.remaining() / 2;
    let mut partial = buf.slice(..upper);
    let _ = partial.get_vi();
    let _ = partial.get_u16();
    assert!(Setup::parse_payload(&mut partial).is_err());
  }
}
