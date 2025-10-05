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

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use bytes::{Buf, BufMut, BytesMut};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::task::yield_now;
use tokio::time::{Instant, sleep_until};
use wtransport::error::StreamReadError;
use wtransport::{RecvStream, SendStream};

use crate::model::control::fetch::Fetch;
use crate::model::control::subscribe::Subscribe;
use crate::model::data::constant::FetchHeaderType;
use crate::model::data::fetch_header::FetchHeader;
use crate::model::data::fetch_object::FetchObject;
use crate::model::data::object::Object;
use crate::model::data::subgroup_header::SubgroupHeader;
use crate::model::data::subgroup_object::SubgroupObject;
use crate::model::error::ParseError;
use tracing::{debug, error, info};

// Timeout for header and subsequent objects
const DATA_STREAM_TIMEOUT: Duration = Duration::from_secs(15);
const MTU_SIZE: usize = 1500; // Standard MTU size

// Stores the context derived from the initial header message
#[derive(Debug, Clone)]
pub enum HeaderInfo {
  Fetch {
    header: FetchHeader,
    fetch_request: Fetch, // Store the original request for context
  },
  Subgroup {
    header: SubgroupHeader,
    // TODO: the following cannot be guaranteed and obtained properly just by track alias
    /*subscribe_request: Subscribe,*/ // Store the original request for context
  },
}

#[derive(Debug, Clone)]
pub struct FetchRequest {
  pub original_request_id: u64,
  pub requested_by: usize, // client id
  pub fetch_request: Fetch,
  pub track_alias: u64, // Track alias for the fetch request
}

#[derive(Debug, Clone)]
pub struct SubscribeRequest {
  pub original_request_id: u64,
  pub requested_by: usize, // connection id
  pub subscribe_request: Subscribe,
}

impl FetchRequest {
  pub fn new(
    original_request_id: u64,
    requested_by: usize,
    fetch_request: Fetch,
    track_alias: u64,
  ) -> Self {
    Self {
      original_request_id,
      requested_by,
      fetch_request,
      track_alias,
    }
  }
}

impl SubscribeRequest {
  pub fn new(original_request_id: u64, requested_by: usize, subscribe_request: Subscribe) -> Self {
    Self {
      original_request_id,
      requested_by,
      subscribe_request,
    }
  }
}

/// # Pseudocode
/// ```rust
/// open-uni
/// let head = FetchHeader::new(..)
/// let data_Stream = SendDataStream::new(send_stream, head).await?;
/// loop{
/// //get the object
/// let object = Object{..}
/// data_stream.send_object().await
/// }
///
/// ```
pub struct SendDataStream {
  send_stream: Arc<Mutex<SendStream>>,
  header_info: HeaderInfo,
}

// TODO: Major issue, can't distinguish from FetchHeader + FetchObject from SubgroupHeader
// Suggestion: SubgroupHeader should start with Request ID and discard track_alias
impl SendDataStream {
  pub async fn new(
    send_stream: Arc<Mutex<SendStream>>,
    header_info: HeaderInfo,
  ) -> Result<Self, ParseError> {
    let mut buf = BytesMut::new();

    match &header_info {
      HeaderInfo::Fetch { header, .. } => {
        buf.extend_from_slice(&header.serialize()?);
      }
      HeaderInfo::Subgroup { header, .. } => {
        buf.extend_from_slice(&header.serialize()?);
      }
    }

    send_stream
      .lock()
      .await
      .write_all(&buf)
      .await
      .map_err(|e| ParseError::Other {
        context: "SendDataStream::new(header write)",
        msg: e.to_string(),
      })?;

    Ok(Self {
      send_stream,
      header_info,
    })
  }

  pub async fn send_object(
    &mut self,
    object: &Object,
    previous_object_id: Option<u64>,
  ) -> Result<(), ParseError> {
    let mut buf = BytesMut::new();
    let object = object.clone();

    match &self.header_info {
      HeaderInfo::Fetch { .. } => {
        let fetch_obj = object.try_into_fetch()?;
        buf.extend_from_slice(&fetch_obj.serialize()?);
      }
      HeaderInfo::Subgroup { header, .. } => {
        let has_extensions = header.header_type.has_extensions();
        let subgroup_obj = object.try_into_subgroup()?;
        buf.extend_from_slice(&subgroup_obj.serialize(previous_object_id, has_extensions)?);
      }
    }

    self
      .send_stream
      .lock()
      .await
      .write_all(&buf)
      .await
      .map_err(|e| ParseError::Other {
        context: "SendDataStream::send_object",
        msg: e.to_string(),
      })?;

    Ok(())
  }

  pub async fn flush(&mut self) -> Result<(), ParseError> {
    debug!("SendDataStream::flush() called");
    self
      .send_stream
      .lock()
      .await
      .flush()
      .await
      .map_err(|e| ParseError::Other {
        context: "SendDataStream::flush",
        msg: e.to_string(),
      })
  }

  pub async fn finish(&mut self) -> Result<(), ParseError> {
    debug!("SendDataStream::finish() called");
    self
      .send_stream
      .lock()
      .await
      .finish()
      .await
      .map_err(|e| ParseError::Other {
        context: "SendDataStream::finish",
        msg: e.to_string(),
      })
  }
}

#[derive(Debug)]
pub enum RecvDataStreamReadError {
  ParseError(ParseError),
  StreamClosed,
}

/// # Pseudocode
/// ```rust
/// accept-uni
/// let pending_fetches: &mut BTreeMap<u64, Fetch> = Session.fetch_requests;
/// let data_Stream = RecvDataStream::new(head, pending_fetches).await?;
/// loop{
/// //get the object
/// let object = data_stream.next_object().await;
/// }
/// ```
pub struct RecvDataStream {
  recv_stream: Arc<Mutex<RecvStream>>,
  header_info: Arc<Mutex<Option<HeaderInfo>>>,
  pending_fetches: Arc<RwLock<BTreeMap<u64, FetchRequest>>>, // Mutable borrow to potentially remove entry
  objects: Arc<RwLock<VecDeque<Object>>>,                    // Buffer for parsed objects
  is_closed: Arc<RwLock<bool>>,                              // Track if the stream is closed
  started_read_task: Arc<Mutex<bool>>,                       // Track if read task has started
  notify: Arc<Notify>,
}

impl RecvDataStream {
  pub fn new(
    recv_stream: RecvStream,
    pending_fetches: Arc<RwLock<BTreeMap<u64, FetchRequest>>>, // Mutable borrow to potentially remove entry
  ) -> Self {
    Self {
      recv_stream: Arc::new(Mutex::new(recv_stream)),
      header_info: Arc::new(Mutex::new(None)), // Initially no header info
      pending_fetches,
      objects: Arc::new(RwLock::new(VecDeque::new())), // Initialize the object buffer
      is_closed: Arc::new(RwLock::new(false)),         // Track if the stream is closed
      started_read_task: Arc::new(Mutex::new(false)),
      notify: Arc::new(Notify::new()),
    }
  }

  pub async fn get_header_info(&self) -> Option<HeaderInfo> {
    debug!("RecvDataStream::get_header_info() called");
    let header_info = self.header_info.lock().await;
    header_info.clone()
  }

  async fn read(
    recv_stream: Arc<Mutex<RecvStream>>,
    is_closed: Arc<RwLock<bool>>,
    the_header_info: Arc<Mutex<Option<HeaderInfo>>>,
    pending_fetches: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
    objects: Arc<RwLock<VecDeque<Object>>>,
    notify: Arc<Notify>,
  ) -> Result<(), RecvDataStreamReadError> {
    let mut header_info = None;
    let mut recv_buf = Box::new([0u8; MTU_SIZE]);
    let mut recv_bytes = BytesMut::new();
    let mut timeout_at = Instant::now() + DATA_STREAM_TIMEOUT;

    let mut previous_object_id: Option<u64> = None;

    loop {
      let bytes_cursor = recv_bytes.clone().freeze();
      if !recv_bytes.is_empty() && header_info.is_none() {
        let is_fetch = recv_bytes[0] == FetchHeaderType::Type0x05 as u8;
        header_info = Self::read_header(
          bytes_cursor,
          is_fetch,
          is_closed.clone(),
          pending_fetches.clone(),
        )
        .await
        .map_err(|e| {
          error!("Failed to parse header: {:?}", e);
          RecvDataStreamReadError::ParseError(e)
        })?;
        let consumed = if let Some((consumed, _)) = header_info.clone() {
          consumed
        } else {
          0
        };
        recv_bytes.advance(consumed);
        *the_header_info.lock().await = Some(header_info.clone().unwrap().1.clone());
      }

      loop {
        let bytes_cursor = recv_bytes.clone().freeze();
        let mut consumed = 0;
        if !recv_bytes.is_empty() {
          let header_info = header_info.clone().unwrap().1;
          let (c, object_id) = Self::read_object(
            bytes_cursor,
            &header_info,
            is_closed.clone(),
            objects.clone(),
            previous_object_id,
          )
          .await
          .map_err(|e| {
            error!("Failed to parse object: {:?}", e);
            RecvDataStreamReadError::ParseError(e)
          })?;
          previous_object_id = object_id;
          consumed = c;

          debug!(
            "previous_object_id: {:?} consumed: {}",
            previous_object_id, consumed
          );

          notify.notify_waiters();
          recv_bytes.advance(consumed);
        }
        if consumed > 0 {
          // We may have more data to parse, so continue reading objects
          continue;
        } else {
          break;
        }
      }

      // Check if the stream is closed
      // If it is closed, notify waiters and return
      if *is_closed.read().await {
        notify.notify_waiters();
        return Ok(());
      }

      let stream = recv_stream.clone();
      let mut stream = stream.lock().await;

      tokio::select! {
          biased;

          _ = sleep_until(timeout_at) => {
            info!("Timeout while waiting for data");
            *is_closed.write().await = true;
            return Err(RecvDataStreamReadError::ParseError(ParseError::Timeout { context: "RecvDataStream::new(header_read)" }));
          }

          read_result = stream.read(&mut recv_buf[..]) => {
            match read_result {
              Ok(Some(n)) => {
                if n > 0 {
                  // debug!("RecvDataStream::read Read {} bytes", n);
                  recv_bytes.put_slice(&recv_buf[..n]);
                  timeout_at = Instant::now() + DATA_STREAM_TIMEOUT;
                } else {
                  // Spurious read, loop again
                  // debug!("RecvDataStream::read Read 0 bytes, spurious read");
                }
              }
              Ok(None) => {
                // If the stream is closed and no more data is available, break or return error
                // otherwise, handle the remaining bytes in the next iteration
                // debug!("RecvDataStream::read Read None (EOF or stream closed)");
                *is_closed.write().await = true;
              }
              Err(e) => {
                debug!("RecvDataStream::read() Read error: {:?}", e);
                *is_closed.write().await = true;
                if e == StreamReadError::NotConnected {
                  return Err(RecvDataStreamReadError::StreamClosed);
                }
                return Err(RecvDataStreamReadError::ParseError(ParseError::Other { context: "RecvDataStream::new(header_read)", msg:e.to_string() }));
              }
            }
          }
      }
    }
  }

  async fn read_header(
    mut bytes_cursor: bytes::Bytes,
    is_fetch: bool,
    is_closed: Arc<RwLock<bool>>,
    pending_fetches: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
  ) -> Result<Option<(usize, HeaderInfo)>, ParseError> {
    debug!("RecvDataStream::read_header() called");
    let original_remaining = bytes_cursor.remaining();
    if is_fetch {
      match FetchHeader::deserialize(&mut bytes_cursor) {
        Ok(fetch_header) => {
          let pending_fetches = pending_fetches.read().await;
          if let Some(fetch_request) = pending_fetches.get(&fetch_header.request_id) {
            let consumed = original_remaining - bytes_cursor.remaining();

            let header_info = HeaderInfo::Fetch {
              header: fetch_header,
              fetch_request: fetch_request.fetch_request.clone(),
            };
            debug!(
              "RecvDataStream::read_header() Parsed FetchHeader: {:?}",
              header_info
            );
            Ok(Some((consumed, header_info)))
          } else {
            // Drop the immutable borrow before calling the async method
            drop(pending_fetches);
            // self.close_stream().await;
            *is_closed.write().await = true;
            Err(ParseError::ProtocolViolation {
              context: "RecvDataStream::new(FetchHeader validation)",
              details: format!(
                "Received FetchHeader for unknown request_id: {}",
                fetch_header.request_id
              ),
            })
          }
        }
        Err(ParseError::NotEnoughBytes { .. }) => {
          Ok(None) // Not enough bytes to parse the header, wait for more data
        }
        Err(e) => {
          *is_closed.write().await = true;
          Err(ParseError::ProtocolViolation {
            context: "RecvDataStream::new(FetchHeader validation)",
            details: e.to_string(),
          })
        }
      }
    } else {
      match SubgroupHeader::deserialize(&mut bytes_cursor) {
        Ok(subgroup_header) => {
          let consumed = original_remaining - bytes_cursor.remaining();
          let header_info = HeaderInfo::Subgroup {
            header: subgroup_header,
            // subscribe_request: subscribe_request.clone().subscribe_request,
          };
          Ok(Some((consumed, header_info)))
        }
        Err(ParseError::NotEnoughBytes { .. }) => {
          Ok(None) // Not enough bytes to parse the header, wait for more data
        }
        Err(e) => {
          *is_closed.write().await = true;
          Err(ParseError::ProtocolViolation {
            context: "RecvDataStream::new(SubgroupHeader validation)",
            details: e.to_string(),
          })
        }
      }
    }
  }

  async fn read_object(
    mut bytes_cursor: bytes::Bytes,
    header_info: &HeaderInfo,
    is_closed: Arc<RwLock<bool>>,
    objects: Arc<RwLock<VecDeque<Object>>>,
    previous_object_id: Option<u64>,
  ) -> Result<(usize, Option<u64>), ParseError> {
    // debug!("RecvDataStream::parse_object() called");

    if !bytes_cursor.is_empty() {
      let original_remaining = bytes_cursor.remaining();

      // debug!("bytes_cursor remaining: {}", original_remaining);

      // get the last object_id and object
      // for fetch objects, the previous object id is not required
      let parse_result = match header_info {
        HeaderInfo::Fetch { .. } => {
          FetchObject::deserialize(&mut bytes_cursor).and_then(|fetch_obj| {
            // TODO: Validation checks fetch objects arriving correctly
            // TODO: Get track alias from fetch_request
            Object::try_from_fetch(fetch_obj, 0).map(|object| (0u64, object))
          })
        }
        HeaderInfo::Subgroup { header, .. } => {
          let has_extensions = header.header_type.has_extensions();
          SubgroupObject::deserialize(&mut bytes_cursor, previous_object_id, has_extensions)
            .and_then(|subgroup_obj| {
              // TODO: Validation checks

              let object_id = subgroup_obj.object_id;

              Object::try_from_subgroup(
                subgroup_obj,
                header.track_alias,
                header.group_id,
                header.subgroup_id,
                header.publisher_priority,
              )
              .map(|object| (object_id, object))
            })
        }
      };

      // debug!("parse_result: {:?}", parse_result);

      match parse_result {
        Ok(object) => {
          let consumed = original_remaining - bytes_cursor.remaining();
          /*
          debug!(
            "consumed: {} Parsed  payload object: {:?}",
            consumed, object
          );
          */
          let mut objects = objects.write().await;
          objects.push_back(object.1);
          Ok((consumed, Some(object.0)))
        }
        Err(ParseError::NotEnoughBytes { .. }) => {
          // Not enough bytes to parse the object, continue reading
          // debug!("Not enough bytes to parse the object, continuing to read...");
          Ok((0, None)) // Indicate that we need more data
        }
        Err(e) => {
          *is_closed.write().await = true;
          Err(ParseError::ProtocolViolation {
            context: "RecvDataStream::next_object(parse_result)",
            details: e.to_string(),
          })
        }
      }
    } else {
      debug!("No bytes available to parse an object, returning false");
      Ok((0, None)) // No bytes to parse, wait for more data
    }
  }

  pub async fn next_object(&self) -> (&Self, Option<Object>) {
    // debug!("RecvDataStream::next_object() called");

    // Start the read task only once
    let mut started = self.started_read_task.lock().await;
    if !*started {
      *started = true;
      let recv_stream = self.recv_stream.clone();
      let is_closed = self.is_closed.clone();
      let pending_fetches = self.pending_fetches.clone();
      let objects = self.objects.clone();
      let header_info = self.header_info.clone();
      let notify = self.notify.clone();
      tokio::spawn(async move {
        match Self::read(
          recv_stream,
          is_closed,
          header_info,
          pending_fetches,
          objects,
          notify,
        )
        .await
        {
          Ok(_) => debug!("RecvDataStream read task completed successfully"),
          Err(e) => {
            error!("RecvDataStream read task encountered an error: {:?}", e);
            // if not connected error, do nothing and return, let the caller handle it
            if matches!(e, RecvDataStreamReadError::StreamClosed) {
              debug!("Stream is closed, returning EOF");
            } else {
              error!("RecvDataStream read task encountered an error: {:?}", e);
            }
          }
        }
      });
    }
    drop(started);

    loop {
      let mut objects = self.objects.write().await;

      if let Some(object) = objects.pop_front() {
        return (self, Some(object));
      }
      drop(objects);

      if *self.is_closed.read().await {
        if self.objects.read().await.is_empty() {
          debug!("Stream is closed, returning EOF");
          return (self, None);
        } else {
          debug!("Stream is closed, but has objects still");
          yield_now().await
        }
      } else {
        // Wait for notification that a new object may be available or the stream is closed.
        self.notify.notified().await;
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::location::Location;
  use crate::model::common::pair::KeyValuePair;
  use crate::model::common::tuple::Tuple;
  use crate::model::control::constant::{FetchType, FilterType, GroupOrder};
  use crate::model::control::fetch::JoiningFetchProps;
  use crate::model::control::{fetch::Fetch, subscribe::Subscribe};
  use crate::model::data::constant::SubgroupHeaderType;
  use bytes::Bytes;
  use std::error::Error;
  use std::sync::Arc;
  use tokio::sync::Mutex;
  use tokio::time::{Duration, sleep};
  use wtransport::endpoint::IntoConnectOptions;
  use wtransport::{ClientConfig, Connection, Endpoint, Identity, RecvStream, SendStream};

  fn make_fetch_header_and_request() -> (FetchHeader, Fetch) {
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
    (FetchHeader { request_id: 161803 }, fetch)
  }

  fn make_fetch_object() -> FetchObject {
    let group_id: u64 = 9;
    let subgroup_id = 144;
    let object_id: u64 = 10;
    let publisher_priority: u8 = 255;
    let extension_headers = Some(vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ]);
    let object_status = None;
    let payload = Some(Bytes::from_static(
      b"01239gjawkk92837aljwdnjwandjnanwdjnajwndkjawndjkanwdkjnawkjddmi",
    ));

    FetchObject {
      group_id,
      subgroup_id,
      object_id,
      publisher_priority,
      extension_headers,
      payload,
      object_status,
    }
  }

  fn make_object_from_fetch(fetch_obj: &FetchObject) -> Object {
    Object::try_from_fetch(fetch_obj.clone(), 0).unwrap()
  }

  #[allow(dead_code)]
  fn make_subgroup_header_and_request() -> (SubgroupHeader, Subscribe) {
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
    let subscribe = Subscribe {
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
    };
    let header_type = SubgroupHeaderType::Type0x15;
    let track_alias = 999;
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
    (subgroup_header, subscribe)
  }

  #[allow(dead_code)]
  fn make_subgroup_object() -> SubgroupObject {
    let object_id: u64 = 10;
    let extension_headers = Some(vec![
      KeyValuePair::try_new_varint(0, 10).unwrap(),
      KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
    ]);
    let object_status = None;
    let payload = Some(Bytes::from_static(b"01239gjawkk92837aldmi"));

    SubgroupObject {
      object_id,
      extension_headers,
      payload,
      object_status,
    }
  }

  #[allow(dead_code)]
  fn make_object_from_subgroup(subgroup_obj: &SubgroupObject, header: &SubgroupHeader) -> Object {
    Object::try_from_subgroup(
      subgroup_obj.clone(),
      header.track_alias,
      header.group_id,
      header.subgroup_id,
      header.publisher_priority,
    )
    .unwrap()
  }

  struct TestSetup {
    client: Connection,
    server: Connection,
  }

  impl TestSetup {
    async fn new() -> Result<Self, Box<dyn Error>> {
      let server_identity = Identity::self_signed(std::iter::once("localhost"))
        .map_err(|e| format!("Failed to create server identity: {e}"))?;
      let server_cert_hash = server_identity.certificate_chain().as_slice()[0].hash();
      let server_config = wtransport::ServerConfig::builder()
        .with_bind_address(
          "127.0.0.1:0"
            .parse()
            .map_err(|e| format!("Failed to parse bind address: {e}"))?,
        )
        .with_identity(server_identity)
        .build();
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
          let incoming = server_endpoint.accept().await;
          let session_request = incoming
            .await
            .map_err(|e| format!("Failed to await session request: {e}"))
            .unwrap();
          let server = session_request.accept().await.unwrap();
          Ok::<_, Box<dyn Error + Send>>(server)
        }
        .await;

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

    async fn create_data_stream_pair(&self) -> Result<(SendStream, RecvStream), Box<dyn Error>> {
      // Client opens a unidirectional stream (SendStream) in a spawned task
      let client = self.client.clone();
      let send_fut = tokio::spawn(async move {
        // The first await returns Result<SendUni, StreamOpeningError>
        // The second await returns Result<SendStream, StreamOpeningError>
        let send_uni = client.open_uni().await.unwrap();
        send_uni.await
      });

      // Server accepts the unidirectional stream (RecvStream) in a spawned task
      let server = self.server.clone();
      let recv_fut = tokio::spawn(async move {
        server
          .accept_uni()
          .await
          .map_err(|e| format!("Failed to accept server uni stream: {e}"))
      });

      // Await both concurrently
      let (send_res, recv_res) = tokio::try_join!(send_fut, recv_fut)?;

      let send = send_res.map_err(|e| format!("Failed to open client uni stream: {e}"))?;
      let recv = recv_res.map_err(|e| format!("Failed to accept server uni stream: {e}"))?;

      Ok((send, recv))
    }
  }

  async fn setup_stream_pair() -> (SendStream, RecvStream) {
    let setup = TestSetup::new()
      .await
      .expect("Failed to setup test transport");
    setup
      .create_data_stream_pair()
      .await
      .expect("Failed to create data stream pair")
  }

  /* TODO: rewrite this test
  #[tokio::test]
  async fn test_send_recv_fetch_object_success() {
    let (send, recv) = setup_stream_pair().await;
    let (fetch_header, fetch_req) = make_fetch_header_and_request();
    let mut pending_fetches: BTreeMap<u64, FetchRequest> = BTreeMap::new();
    pending_fetches.insert(
      fetch_req.request_id,
      FetchRequest {
        original_request_id: fetch_req.request_id,
        requested_by: 1,
        fetch_request: fetch_req.clone(),
        track_alias: 1,
      },
    );

    // Sender
    let mut sender = SendDataStream::new(
      Arc::new(Mutex::new(send)),
      HeaderInfo::Fetch {
        header: fetch_header,
        fetch_request: fetch_req.clone(),
      },
    )
    .await
    .unwrap();

    let fetch_obj = make_fetch_object();
    let object = make_object_from_fetch(&fetch_obj);

    let pending_fetches = Arc::new(RwLock::new(pending_fetches));

    // Receiver
    let receiver = RecvDataStream::new(Arc::new(Mutex::new(recv)), pending_fetches.clone());

    sender.send_object(&object).await.unwrap();
    let received = receiver.next_object().await.1.unwrap();

    assert_eq!(object, received);
  }
  */

  /* TODO: rewrite this test
  #[tokio::test]
  async fn test_send_recv_subgroup_object_success() {
    let (send, recv) = setup_stream_pair().await;
    let (subgroup_header, _) = make_subgroup_header_and_request();
    let pending_fetches = BTreeMap::new();

    let mut sender = SendDataStream::new(
      Arc::new(Mutex::new(send)),
      HeaderInfo::Subgroup {
        header: subgroup_header,
        /* subscribe_request: subscribe_req.clone(), */
      },
    )
    .await
    .unwrap();

    let subgroup_obj = make_subgroup_object();
    let object = make_object_from_subgroup(&subgroup_obj, &subgroup_header);

    let pending_fetches = Arc::new(RwLock::new(pending_fetches));

    let receiver = RecvDataStream::new(Arc::new(Mutex::new(recv)), pending_fetches.clone());

    sender.send_object(&object).await.unwrap();
    let received = receiver.next_object().await.1.unwrap();

    assert_eq!(object, received);
  }
  */

  /* TODO: rewrite this test
  #[tokio::test]
  async fn test_timeout_on_header() {
    let (_send, recv) = setup_stream_pair().await;
    let mut pending_fetches = Arc::new(RwLock::new(BTreeMap::new()));
    let mut pending_subscribes = Arc::new(RwLock::new(BTreeMap::new()));
    // Don't send any header, just wait for timeout
    let result = RecvDataStream::new(
      Arc::new(Mutex::new(recv)),
      pending_fetches,
      pending_subscribes,
    );
    match result {
      Err(ParseError::Timeout { .. }) => {}
      _ => panic!("Should timeout"),
    }
  }
  */

  /*
  #[tokio::test]
  async fn test_partial_object_timeout() {
    let (send, recv) = setup_stream_pair().await;
    let (fetch_header, fetch_req) = make_fetch_header_and_request();
    let mut pending_fetches = BTreeMap::new();
    pending_fetches.insert(
      fetch_req.request_id,
      FetchRequest {
        request_id: fetch_req.request_id,
        requested_by: 1,
        fetch_request: fetch_req.clone(),
        track_alias: 1,
      },
    );
    let mut pending_subscribes = BTreeMap::new();
    // Send only the header, not the object
    let mut _sender = SendDataStream::new(
      Arc::new(Mutex::new(send)),
      HeaderInfo::Fetch {
        header: fetch_header.clone(),
        fetch_request: fetch_req.clone(),
      },
    )
    .await
    .unwrap();
    let pending_fetches = Arc::new(RwLock::new(pending_fetches));
    let pending_subscribes = Arc::new(RwLock::new(pending_subscribes));
    let mut receiver = RecvDataStream::new(
      Arc::new(Mutex::new(recv)),
      pending_fetches,
      pending_subscribes,
    );
    // Don't send any object, just wait for timeout
    let result = receiver.next_object().await;
    match result {
      Err(ParseError::Timeout { .. }) => {}
      other => panic!("Expected timeout, got {:?}", other),
    }
  }
  */
  #[tokio::test]
  async fn test_partial_object_completion() {
    let (send, recv) = setup_stream_pair().await;
    let (fetch_header, fetch_req) = make_fetch_header_and_request();
    let mut pending_fetches = BTreeMap::new();
    pending_fetches.insert(
      fetch_req.request_id,
      FetchRequest {
        original_request_id: fetch_req.request_id,
        requested_by: 1,
        fetch_request: fetch_req.clone(),
        track_alias: 1,
      },
    );

    let sender = SendDataStream::new(
      Arc::new(Mutex::new(send)),
      HeaderInfo::Fetch {
        header: fetch_header,
        fetch_request: fetch_req.clone(),
      },
    )
    .await
    .unwrap();

    let fetch_obj = make_fetch_object();
    let object = make_object_from_fetch(&fetch_obj);

    let pending_fetches = Arc::new(RwLock::new(pending_fetches));

    let receiver = RecvDataStream::new(recv, pending_fetches);

    // Serialize object and send in two parts
    let bytes = fetch_obj.serialize().unwrap();
    let half = bytes.len() / 2;
    let first_half = &bytes[..half];
    let second_half = &bytes[half..];

    // Send first half
    sender
      .send_stream
      .lock()
      .await
      .write_all(first_half)
      .await
      .unwrap();

    // Spawn a task to send the second half after a delay
    let second_half = second_half.to_vec();
    tokio::spawn({
      let send_stream = sender.send_stream.clone();
      async move {
        sleep(Duration::from_millis(100)).await;
        let mut s = send_stream.lock().await;
        s.write_all(&second_half).await.unwrap();
      }
    });

    let received = receiver.next_object().await.1.unwrap();
    assert_eq!(object, received);
  }
}
