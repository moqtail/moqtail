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

use bytes::{Buf, BufMut, BytesMut};
use tokio::time::{Duration, Instant, sleep_until};
use tracing::{error, info, warn};
use wtransport::error::StreamReadError;
use wtransport::{RecvStream, SendStream};

use crate::model::control::control_message::{ControlMessage, ControlMessageTrait};
use crate::model::error::{ParseError, TerminationCode};

const CONTROL_MESSAGE_TIMEOUT: Duration = Duration::from_secs(5);

const MTU_SIZE: usize = 1500; // Standard MTU size, max 2^16-1

pub struct ControlStreamHandler {
  send: SendStream,
  recv: RecvStream,
  recv_bytes: BytesMut,
  recv_buf: Box<[u8; MTU_SIZE]>,
  partial_message_deadline: Option<Instant>,
}

impl ControlStreamHandler {
  pub fn new(send: SendStream, recv: RecvStream) -> Self {
    Self {
      send,
      recv,
      recv_bytes: BytesMut::new(),
      recv_buf: Box::new([0; MTU_SIZE]),
      partial_message_deadline: None,
    }
  }

  pub async fn send(&mut self, message: &ControlMessage) -> Result<(), TerminationCode> {
    let bytes = message
      .serialize()
      .map_err(|_| TerminationCode::InternalError)?;
    if (self.send.write_all(&bytes).await).is_err() {
      warn!("Error sending message: {:?}", message);
      return Err(TerminationCode::InternalError);
    }
    Ok(())
  }

  // TODO: refactor here, implement Serializable trait for ControlMessage
  pub async fn send_impl(
    &mut self,
    message: &impl ControlMessageTrait,
  ) -> Result<(), TerminationCode> {
    let bytes = message
      .serialize()
      .map_err(|_| TerminationCode::InternalError)?;
    if (self.send.write_all(&bytes).await).is_err() {
      warn!("Error sending (send_impl) message: {:?}", message);
      return Err(TerminationCode::InternalError);
    }
    Ok(())
  }

  pub async fn next_message(&mut self) -> Result<ControlMessage, TerminationCode> {
    loop {
      if !self.recv_bytes.is_empty() {
        let mut bytes = self.recv_bytes.clone().freeze();
        let original_remaining = bytes.remaining();

        match ControlMessage::deserialize(&mut bytes) {
          Ok(msg) => {
            let consumed = original_remaining - bytes.remaining();
            self.recv_bytes.advance(consumed);

            self.partial_message_deadline = None;

            return Ok(msg);
          }
          Err(ParseError::ProtocolViolation { .. }) => {
            return Err(TerminationCode::ProtocolViolation);
          }

          Err(ParseError::NotEnoughBytes { .. }) => {
            if self.partial_message_deadline.is_none() {
              self.partial_message_deadline = Some(Instant::now() + CONTROL_MESSAGE_TIMEOUT);
            }
          }
          _ => {}
        }
      }

      // Read more data, either with or without deadline
      self.read_more_data().await?;
    }
  }

  /// Read more data from the stream, handling deadlines if set
  async fn read_more_data(&mut self) -> Result<(), TerminationCode> {
    // Check if we've timed out on a partial message
    if let Some(deadline) = self.partial_message_deadline {
      if Instant::now() >= deadline {
        self.partial_message_deadline = None;
        self.recv_bytes.clear();
        return Err(TerminationCode::ControlMessageTimeout);
      }

      tokio::select! {
        biased;

        _ = sleep_until(deadline) => {
          info!("Control message timeout reached");
          self.partial_message_deadline = None;
          self.recv_bytes.clear();
          Err(TerminationCode::ControlMessageTimeout)
        }

        res = self.recv.read(&mut self.recv_buf[..]) => {
          self.handle_read_result(res, true)
        }
      }
    } else {
      // No partial message deadline is set, so just wait for any data.
      let res = self.recv.read(&mut self.recv_buf[..]).await;
      self.handle_read_result(res, false)
    }
  }

  /// Handle the result of a stream read operation
  fn handle_read_result(
    &mut self,
    res: Result<Option<usize>, wtransport::error::StreamReadError>,
    is_partial_message: bool,
  ) -> Result<(), TerminationCode> {
    match res {
      Ok(Some(n)) => {
        if n > 0 {
          // Successfully read some new data. Append it and loop back to try parsing again.
          self.recv_bytes.put_slice(&self.recv_buf[..n]);
        }
        // If n == 0, reading 0 bytes might indicate a stream state change, but often
        // it's benign. We simply loop back and try parsing/reading again.
        // If the stream *was* closed, the next read attempt should yield Ok(None) or Err.
        Ok(())
      }
      Ok(None) => {
        if is_partial_message {
          info!("Stream closed cleanly while waiting for partial message");
        } else {
          warn!("Stream closed cleanly while waiting for data");
        }
        // The stream was closed cleanly by the peer.
        Err(TerminationCode::InternalError)
      }
      Err(e) => {
        match e {
          StreamReadError::NotConnected => {
            info!("Client disconnected while reading control stream");
            // Client has disconnected - this is not an error, return NoError
            // so the caller knows not to try closing the connection
            Err(TerminationCode::NoError)
          }
          _ => {
            if is_partial_message {
              info!("Error reading from stream: {:?}", e);
            } else {
              error!("Error reading from stream: {:?}", e);
            }
            Err(TerminationCode::InternalError)
          }
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::location::Location;
  use crate::model::common::pair::KeyValuePair;
  use crate::model::common::reason_phrase::ReasonPhrase;
  use crate::model::common::tuple::Tuple;
  use crate::model::common::varint::BufMutVarIntExt;
  use crate::model::control::client_setup::ClientSetup;
  use crate::model::control::constant::{ControlMessageType, FilterType, GroupOrder};
  use crate::model::control::constant::{DRAFT_14, PublishNamespaceErrorCode};
  use crate::model::control::publish_namespace::PublishNamespace;
  use crate::model::control::publish_namespace_cancel::PublishNamespaceCancel;
  use crate::model::control::publish_namespace_ok::PublishNamespaceOk;
  use crate::model::control::server_setup::ServerSetup;
  use crate::model::control::subscribe::Subscribe;
  use crate::model::control::subscribe_ok::SubscribeOk;
  use bytes::Bytes;
  use std::error::Error;
  use std::sync::Arc;
  use tokio::sync::Mutex;
  use tokio::time::sleep;
  use wtransport::endpoint::IntoConnectOptions;
  use wtransport::{ClientConfig, Connection, Endpoint, Identity};

  struct TestSetup {
    client: Connection,
    server: Connection,
  }

  impl TestSetup {
    async fn new() -> Result<Self, Box<dyn Error>> {
      // Create server identity (self-signed certificate)
      let server_identity = Identity::self_signed(std::iter::once("localhost"))
        .map_err(|e| format!("Failed to create server identity: {e}"))?;

      // Get the certificate hash from the server identity
      let server_cert_hash = server_identity.certificate_chain().as_slice()[0].hash();

      // Create server configuration with the identity
      let server_config = wtransport::ServerConfig::builder()
        .with_bind_address(
          "127.0.0.1:0"
            .parse()
            .map_err(|e| format!("Failed to parse bind address: {e}"))?,
        )
        .with_identity(server_identity)
        .build();

      // Create and start the server endpoint
      let server_endpoint = Endpoint::server(server_config)
        .map_err(|e| format!("Failed to create server endpoint: {e}"))?;
      let server_addr = server_endpoint
        .local_addr()
        .map_err(|e| format!("Failed to get server local address: {e}"))?;

      // Create a channel to get the server connection from the spawned task
      let (tx, rx) = tokio::sync::oneshot::channel();

      // Spawn a task to handle the server connection acceptance
      tokio::spawn(async move {
        let result = async {
          // Accept an incoming connection
          let incoming = server_endpoint.accept().await;

          // Await the session request
          let session_request = incoming
            .await
            .map_err(|e| format!("Failed to await session request: {e}"))
            .unwrap();

          let server = session_request.accept().await.unwrap();
          Ok::<_, Box<dyn Error + Send>>(server)
        }
        .await;

        // Send result back through the channel, log errors if sending fails
        if tx.send(result).is_err() {
          eprintln!("Failed to send server connection result back through the channel");
        }
      });

      // Create client configuration with server's certificate hash for verification
      let client_config = ClientConfig::builder()
        .with_bind_default()
        .with_server_certificate_hashes(vec![server_cert_hash])
        .build();

      let client_endpoint = Endpoint::client(client_config)
        .map_err(|e| format!("Failed to create client endpoint: {e}"))?;

      // Start client connection concurrently while server is accepting
      let client = client_endpoint
        .connect(
          format!("https://{}:{}", server_addr.ip(), server_addr.port())
            .as_str()
            .into_options(),
        )
        .await
        .map_err(|e| format!("Client connection failed: {e}"))?;

      // Wait for the server connection from the spawned task
      let server = rx
        .await
        .map_err(|_| "Server task failed to send connection back")?
        .map_err(|e| format!("Server connection error: {e}"))?;

      Ok(Self { client, server })
    }

    async fn create_control_plane(
      &self,
    ) -> Result<(ControlStreamHandler, SendStream), Box<dyn Error>> {
      let (client_send, client_recv) = match self.client.open_bi().await {
        Ok(stream_fut) => match stream_fut.await {
          Ok((send, recv)) => (send, recv),
          Err(e) => return Err(format!("Failed to await client stream: {e}").into()),
        },
        Err(e) => return Err(format!("Failed to open client stream: {e}").into()),
      };

      let (server_send, _) = self
        .server
        .accept_bi()
        .await
        .map_err(|e| format!("Failed to accept server stream: {e}"))?;

      let plane = ControlStreamHandler::new(client_send, client_recv);
      Ok((plane, server_send))
    }
  }

  fn create_test_publish_namespace() -> PublishNamespace {
    let request_id = 12345;
    let track_namespace = Tuple::from_utf8_path("god/dayyum");
    let parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ];
    PublishNamespace {
      request_id,
      track_namespace,
      parameters,
    }
  }

  fn create_test_announce_ok() -> PublishNamespaceOk {
    let request_id = 12345;
    PublishNamespaceOk { request_id }
  }

  fn create_test_announce_cancel() -> PublishNamespaceCancel {
    let error_code = PublishNamespaceErrorCode::InternalError;
    let reason_phrase = ReasonPhrase::try_new("bomboclad".to_string()).unwrap();
    let track_namespace = Tuple::from_utf8_path("another/valid/track/namespace");
    PublishNamespaceCancel {
      error_code,
      reason_phrase,
      track_namespace,
    }
  }

  fn create_test_subscribe() -> Subscribe {
    let request_id = 128242;
    let track_namespace = Tuple::from_utf8_path("nein/nein/nein");
    let track_name = "${Name}".to_string();
    let subscriber_priority = 31;
    let group_order = GroupOrder::Original;
    let forward = true;
    let filter_type = FilterType::AbsoluteRange;
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let end_group = 25;
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"I'll sync you up")).unwrap(),
    ];
    Subscribe {
      request_id,
      track_namespace,
      track_name,
      subscriber_priority,
      group_order,
      forward,
      filter_type,
      start_location: Some(start_location),
      end_group: Some(end_group),
      subscribe_parameters,
    }
  }

  fn create_test_subscribe_ok() -> SubscribeOk {
    let request_id = 145136;
    let track_alias = 999;
    let expires = 16;
    let group_order = GroupOrder::Ascending;
    let content_exists = true;
    let largest_location = Location {
      group: 34,
      object: 0,
    };
    let subscribe_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"9 gifted subs from Dr.Doofishtein"))
        .unwrap(),
    ];
    SubscribeOk {
      request_id,
      track_alias,
      expires,
      group_order,
      content_exists,
      largest_location: Some(largest_location),
      subscribe_parameters: Some(subscribe_parameters),
    }
  }

  fn create_test_client_setup() -> ClientSetup {
    let supported_versions = vec![12345, DRAFT_14];
    let setup_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"Set me up!")).unwrap(),
    ];
    ClientSetup {
      supported_versions,
      setup_parameters,
    }
  }

  fn create_test_server_setup() -> ServerSetup {
    let selected_version = DRAFT_14;
    let setup_parameters = vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"Set me up!")).unwrap(),
    ];
    ServerSetup {
      selected_version,
      setup_parameters,
    }
  }

  #[tokio::test]
  async fn test_connection_setup() -> Result<(), Box<dyn Error>> {
    let setup = TestSetup::new().await?;

    // Test that we can create a bidirectional stream
    let (client_send, _) = setup
      .client
      .open_bi()
      .await?
      .await
      .map_err(|e| format!("Failed to open bidirectional stream: {e}"))?;

    let mut client_send = client_send;
    let (_, mut server_recv) = setup
      .server
      .accept_bi()
      .await
      .map_err(|e| format!("Failed to accept bidirectional stream: {e}"))?;

    // Test basic data transfer
    client_send.write_all(&[1, 2, 3, 4]).await?;
    let mut buf = [0; 4];
    server_recv.read_exact(&mut buf).await?;

    assert_eq!(buf, [1, 2, 3, 4]);

    Ok(())
  }

  #[tokio::test]
  async fn test_message_timeout() -> Result<(), Box<dyn Error>> {
    let setup = TestSetup::new().await?;
    let (mut plane, mut server_send) = setup.create_control_plane().await?;
    let mut bytes = BytesMut::new();
    bytes.put_vi(ControlMessageType::PublishNamespace)?;
    let bytes = bytes.freeze();
    // Send only the message type to cause a partial message
    server_send.write_all(&bytes).await?; // ANNOUNCE_CANCEL type

    // Should timeout after receiving partial message
    match plane.next_message().await {
      Err(TerminationCode::ControlMessageTimeout) => Ok(()),
      other => panic!("Expected timeout, got {other:?}"),
    }
  }

  #[tokio::test]
  async fn test_successful_message() -> Result<(), Box<dyn Error>> {
    let setup = TestSetup::new().await?;
    let (mut plane, mut server_send) = setup.create_control_plane().await?;

    // Create and send a valid message using helper
    let announce = Box::new(create_test_publish_namespace());
    let msg = announce.clone(); // Clone announce for assertion

    let bytes = msg.serialize().unwrap();
    server_send.write_all(&bytes).await?;

    let received = plane.next_message().await.unwrap();

    // Assert based on the original created struct, not the enum variant
    match received {
      ControlMessage::PublishNamespace(rec_announce) => assert_eq!(rec_announce, announce),
      _ => panic!("Received incorrect message type"),
    }

    Ok(())
  }

  #[tokio::test]
  async fn test_partial_message_completion() -> Result<(), Box<dyn Error>> {
    let setup = TestSetup::new().await?;
    let (mut plane, mut server_send) = setup.create_control_plane().await?;

    // Create a valid message using helper
    let announce_cancel = Box::new(create_test_announce_cancel());
    let msg = ControlMessage::PublishNamespaceCancel(announce_cancel.clone()); // Clone for assertion
    let bytes = msg.serialize().unwrap();

    // Send first half
    let half = bytes.len() / 2;
    server_send.write_all(&bytes[..half]).await?;

    // Send second half after a small delay
    let remaining_bytes = bytes[half..].to_vec();
    tokio::spawn(async move {
      sleep(Duration::from_millis(100)).await;
      server_send.write_all(&remaining_bytes).await.unwrap();
    });

    // Should successfully receive the complete message
    let received = plane.next_message().await.unwrap();
    match received {
      ControlMessage::PublishNamespaceCancel(rec_cancel) => assert_eq!(rec_cancel, announce_cancel),
      _ => panic!("Received incorrect message type"),
    }

    Ok(())
  }

  #[tokio::test]
  async fn test_multiple_messages() -> Result<(), Box<dyn Error>> {
    let setup = TestSetup::new().await?;
    let (mut plane, server_send) = setup.create_control_plane().await?;
    // Use Arc<Mutex<>> for SendStream to share it with the spawned task
    let server_send = Arc::new(Mutex::new(server_send));

    // Create messages using helpers
    let announce1 = Box::new(create_test_publish_namespace());
    let announce_ok1 = Box::new(create_test_announce_ok());
    let subscribe1 = Box::new(create_test_subscribe());
    let subscribe_ok1 = Box::new(create_test_subscribe_ok());
    let announce_cancel1 = Box::new(create_test_announce_cancel());
    let client_setup = Box::new(create_test_client_setup());
    let server_setup = Box::new(create_test_server_setup());

    let messages_to_send = vec![
      ControlMessage::ClientSetup(client_setup),
      ControlMessage::ServerSetup(server_setup),
      ControlMessage::PublishNamespace(announce1),
      ControlMessage::PublishNamespaceOk(announce_ok1),
      ControlMessage::Subscribe(subscribe1),
      ControlMessage::SubscribeOk(subscribe_ok1),
      ControlMessage::PublishNamespaceCancel(announce_cancel1),
    ];

    // Clone messages for sending task
    let messages_clone = messages_to_send.clone();
    let server_send_clone = Arc::clone(&server_send);

    // Spawn a task to send messages from the server side
    tokio::spawn(async move {
      let mut sender = server_send_clone.lock().await;
      for msg in messages_clone {
        let bytes = msg.serialize().unwrap();
        if sender.write_all(&bytes).await.is_err() {
          eprintln!("Error sending message in test task");
          return; // Stop sending if there's an error
        }
        // Small delay to simulate real-world conditions and prevent clumping
        sleep(Duration::from_millis(10)).await;
      }
      // Optionally close the stream or just drop the sender
      // sender.finish().await.ok();
    });

    // Receive messages on the client side and verify
    for expected_msg in messages_to_send {
      let received_msg = plane.next_message().await.unwrap();
      assert_eq!(
        received_msg, expected_msg,
        "Mismatch between sent and received message.\nExpected: {expected_msg:?}\nReceived: {received_msg:?}"
      );
    }

    Ok(())
  }
}
