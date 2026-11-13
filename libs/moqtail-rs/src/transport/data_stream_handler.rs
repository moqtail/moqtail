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

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Duration;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::task::yield_now;
use tokio::time::{Instant, sleep_until};

use crate::model::common::location::Location;
use crate::model::control::constant::GroupOrder;
use crate::model::control::fetch::Fetch;
use crate::model::control::subscribe::Subscribe;
use crate::model::data::constant::{
  DEFAULT_PUBLISHER_PRIORITY, FetchHeaderType, ObjectForwardingPreference,
};
use crate::model::data::fetch_header::FetchHeader;
use crate::model::data::fetch_object::{
  EndOfRangeKind, FetchObject, FetchObjectContext, FetchObjectPayload,
};
use crate::model::data::object::Object;
use crate::model::data::subgroup_header::SubgroupHeader;
use crate::model::data::subgroup_object::SubgroupObject;
use crate::model::error::ParseError;
use crate::model::property::object_property::validate_prior_gaps;
use crate::transport::connection::{TransportReadError, TransportRecvStream, TransportSendStream};
use crate::transport::qlog;
use tracing::{debug, error, info, trace};

type ObjectParseResult = Result<
  (
    Option<u64>,
    Option<FetchObjectContext>,
    Option<DataStreamItem>,
  ),
  ParseError,
>;

/// What a data stream carries, in the order it carries it.
#[derive(Debug, Clone, PartialEq)]
pub enum DataStreamItem {
  Object(Object),
  /// A fetch stream's End of Range marker: every Location after the previous Object,
  /// if any, up to and including `location`, is of the marked kind.
  EndOfRange {
    kind: EndOfRangeKind,
    location: Location,
  },
}

// Timeout for header and subsequent objects
const DATA_STREAM_TIMEOUT: Duration = Duration::from_secs(15);
const MTU_SIZE: usize = 1500; // Standard MTU size

/// Lends the bytes received so far to a parser without copying them.
fn lend(buf: &mut BytesMut) -> Bytes {
  std::mem::take(buf).freeze()
}

/// Takes the lent bytes back without the first `consumed`. The rest is copied only
/// if a parsed payload still shares the memory.
fn reclaim(mut view: Bytes, consumed: usize) -> BytesMut {
  view.advance(consumed);
  view
    .try_into_mut()
    .unwrap_or_else(|shared| BytesMut::from(&shared[..]))
}

#[derive(Default)]
struct FetchReadState {
  prev_ctx: Option<FetchObjectContext>,
  subgroup_priorities: FetchSubgroupPriorityState,
}

#[derive(Default)]
struct FetchSubgroupPriorityState {
  group_id: Option<u64>,
  priorities: HashMap<u64, u8>,
}

impl FetchSubgroupPriorityState {
  fn validate(&mut self, payload: &FetchObjectPayload) -> Result<(), ParseError> {
    if self.group_id != Some(payload.group_id) {
      self.group_id = Some(payload.group_id);
      self.priorities.clear();
    }

    if payload.forwarding_preference != ObjectForwardingPreference::Subgroup {
      return Ok(());
    }

    match self
      .priorities
      .insert(payload.subgroup_id, payload.publisher_priority)
    {
      Some(previous_priority) if previous_priority != payload.publisher_priority => {
        Err(ParseError::ProtocolViolation {
          context: "RecvDataStream::read_object(fetch_malformed_track)",
          details: format!(
            "FETCH object for subgroup_id {} changed publisher_priority from {} to {}",
            payload.subgroup_id, previous_priority, payload.publisher_priority
          ),
        })
      }
      _ => Ok(()),
    }
  }
}

// Stores the context derived from the initial header message
/// The Group Order the objects on a fetch stream are encoded with.
///
/// A fill fetch stream has no FETCH behind it, so the order is the one a FETCH
/// defaults to when it says nothing. A publisher that fills in Descending order
/// would need the subscription's own Group Order, which this layer does not see.
fn fetch_group_order(fetch_request: Option<&Fetch>) -> GroupOrder {
  fetch_request.map_or(GroupOrder::Ascending, |fetch| fetch.group_order())
}

#[derive(Debug, Clone)]
pub enum HeaderInfo {
  Fetch {
    header: FetchHeader,
    /// The FETCH this stream answers, when the Request ID names one. A fill fetch
    /// stream carries the same header but names the SUBSCRIBE or REQUEST_UPDATE
    /// that asked for the fill, so there is no FETCH to point at.
    fetch_request: Option<Fetch>,
  },
  Subgroup {
    header: SubgroupHeader,
    // TODO: the following cannot be guaranteed and obtained properly just by track alias
    /*subscribe_request: Subscribe,*/ // Store the original request for context
  },
}

// TODO: Consider merging FetchRequest and SubscribeRequest into a single enum if needed
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
  pub requested_by: usize,                               // connection id
  pub original_subscribe_request: Subscribe, // the subscribe request sent by the subscriber
  pub subscribe_request_to_publisher: Option<Subscribe>, // the subscribe request sent to the publisher (if applicable)
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
  pub fn new(
    original_request_id: u64,
    requested_by: usize,
    original_subscribe_request: Subscribe,
    subscribe_request_to_publisher: Option<Subscribe>,
  ) -> Self {
    Self {
      original_request_id,
      requested_by,
      original_subscribe_request,
      subscribe_request_to_publisher,
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
  send_stream: Arc<Mutex<TransportSendStream>>,
  header_info: HeaderInfo,
  fetch_prev_ctx: Option<FetchObjectContext>,
}

// TODO: Major issue, can't distinguish from FetchHeader + FetchObject from SubgroupHeader
// Suggestion: SubgroupHeader should start with Request ID and discard track_alias
impl SendDataStream {
  pub async fn new(
    send_stream: Arc<Mutex<TransportSendStream>>,
    header_info: HeaderInfo,
  ) -> Result<Self, ParseError> {
    let mut buf = BytesMut::new();

    match &header_info {
      HeaderInfo::Fetch { header, .. } => {
        buf.extend_from_slice(&header.serialize()?);
      }
      HeaderInfo::Subgroup { header, .. } => {
        buf.extend_from_slice(&header.serialize(None)?);
      }
    }

    let mut stream = send_stream.lock().await;
    stream
      .write_all(&buf)
      .await
      .map_err(|e| ParseError::Other {
        context: "SendDataStream::new(header write)",
        msg: e.to_string(),
      })?;
    // A client has one connection, so its qlog connection is 0.
    qlog::data_stream_header(0, stream.id(), true, &buf);
    drop(stream);

    Ok(Self {
      send_stream,
      header_info,
      fetch_prev_ctx: None,
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
      HeaderInfo::Fetch { fetch_request, .. } => {
        let group_order = fetch_group_order(fetch_request.as_ref());
        let payload = object.try_into_fetch()?;
        let fetch_obj = FetchObject::Object(payload);
        buf.extend_from_slice(&fetch_obj.serialize(self.fetch_prev_ctx.as_ref(), group_order)?);
        self.fetch_prev_ctx = Some(fetch_obj.next_context(self.fetch_prev_ctx.as_ref()));
      }
      HeaderInfo::Subgroup { header, .. } => {
        let has_properties = header.header_type.has_properties();
        let subgroup_obj = object.try_into_subgroup()?;
        buf.extend_from_slice(&subgroup_obj.serialize(previous_object_id, has_properties)?);
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

  /// Writes an End of Range marker on a fetch stream: every Location after the last
  /// Object written, if any, up to and including `location`, is of `kind`. The Object
  /// written next is encoded against the marker's Location.
  pub async fn send_end_of_range(
    &mut self,
    kind: EndOfRangeKind,
    location: &Location,
  ) -> Result<(), ParseError> {
    let HeaderInfo::Fetch { .. } = &self.header_info else {
      return Err(ParseError::Other {
        context: "SendDataStream::send_end_of_range",
        msg: "End of Range markers are only for fetch streams".to_string(),
      });
    };
    let marker = FetchObject::EndOfRange {
      kind,
      group_id: location.group,
      object_id: location.object,
    };
    // A marker's encoding does not depend on the Group Order.
    let buf = marker.serialize(self.fetch_prev_ctx.as_ref(), GroupOrder::Ascending)?;
    self.fetch_prev_ctx = Some(marker.next_context(self.fetch_prev_ctx.as_ref()));

    self
      .send_stream
      .lock()
      .await
      .write_all(&buf)
      .await
      .map_err(|e| ParseError::Other {
        context: "SendDataStream::send_end_of_range",
        msg: e.to_string(),
      })
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
  recv_stream: Arc<Mutex<TransportRecvStream>>,
  header_info: Arc<Mutex<Option<HeaderInfo>>>,
  pending_fetches: Arc<RwLock<BTreeMap<u64, FetchRequest>>>, // Mutable borrow to potentially remove entry
  items: Arc<RwLock<VecDeque<DataStreamItem>>>,              // Parsed and not yet taken
  is_closed: Arc<AtomicBool>,
  malformed_track: Arc<AtomicBool>,
  started_read_task: Arc<AtomicBool>,
  /// How the read task ended: `READ_RUNNING` until it does.
  read_outcome: Arc<AtomicU8>,
  notify: Arc<Notify>,
  /// The connection qlog events of this stream belong to.
  qlog_conn: usize,
}

const READ_RUNNING: u8 = 0;
const READ_FINISHED: u8 = 1;
const READ_ABORTED: u8 = 2;

impl RecvDataStream {
  pub fn new(
    recv_stream: TransportRecvStream,
    pending_fetches: Arc<RwLock<BTreeMap<u64, FetchRequest>>>, // Mutable borrow to potentially remove entry
  ) -> Self {
    Self {
      recv_stream: Arc::new(Mutex::new(recv_stream)),
      header_info: Arc::new(Mutex::new(None)), // Initially no header info
      pending_fetches,
      items: Arc::new(RwLock::new(VecDeque::new())),
      is_closed: Arc::new(AtomicBool::new(false)),
      malformed_track: Arc::new(AtomicBool::new(false)),
      started_read_task: Arc::new(AtomicBool::new(false)),
      read_outcome: Arc::new(AtomicU8::new(READ_RUNNING)),
      notify: Arc::new(Notify::new()),
      qlog_conn: 0,
    }
  }

  /// Names the peer connection for qlog events; 0 when unset, as for a client.
  pub fn with_peer_id(mut self, peer_id: usize) -> Self {
    self.qlog_conn = peer_id;
    self
  }

  pub fn is_malformed_track(&self) -> bool {
    self.malformed_track.load(Ordering::Relaxed)
  }

  /// Whether the stream ended without a clean FIN: reset by the peer, idle past the
  /// timeout, or unparseable. Objects it was still to carry never arrived. Settled
  /// once `next_object` has returned `None`.
  pub fn was_aborted(&self) -> bool {
    self.read_outcome.load(Ordering::Acquire) == READ_ABORTED
  }

  pub async fn get_header_info(&self) -> Option<HeaderInfo> {
    debug!("RecvDataStream::get_header_info() called");
    let header_info = self.header_info.lock().await;
    header_info.clone()
  }

  #[allow(clippy::too_many_arguments)]
  async fn read(
    recv_stream: Arc<Mutex<TransportRecvStream>>,
    is_closed: Arc<AtomicBool>,
    the_header_info: Arc<Mutex<Option<HeaderInfo>>>,
    pending_fetches: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
    items: Arc<RwLock<VecDeque<DataStreamItem>>>,
    malformed_track: Arc<AtomicBool>,
    notify: Arc<Notify>,
    qlog_conn: usize,
  ) -> Result<(), RecvDataStreamReadError> {
    let mut header_info = None;
    let mut recv_buf = Box::new([0u8; MTU_SIZE]);
    let mut recv_bytes = BytesMut::new();
    let mut timeout_at = Instant::now() + DATA_STREAM_TIMEOUT;

    let mut previous_object_id: Option<u64> = None;
    let mut fetch_state = FetchReadState::default();

    loop {
      if !recv_bytes.is_empty() && header_info.is_none() {
        let is_fetch = recv_bytes[0] == FetchHeaderType::Type0x05 as u8;
        let view = lend(&mut recv_bytes);
        let parsed = Self::read_header(
          view.clone(),
          is_fetch,
          is_closed.clone(),
          pending_fetches.clone(),
        )
        .await;
        let consumed = match &parsed {
          Ok(Some((consumed, _))) => *consumed,
          _ => 0,
        };
        if consumed > 0 && qlog::enabled() {
          let stream_id = recv_stream.lock().await.id();
          qlog::data_stream_header(qlog_conn, stream_id, false, &view[..consumed]);
        }
        recv_bytes = reclaim(view, consumed);
        header_info = parsed.map_err(|e| {
          error!("Failed to parse header: {:?}", e);
          RecvDataStreamReadError::ParseError(e)
        })?;
        // Nothing yet when the header is split across reads: wait for the rest of it.
        if let Some((_, header)) = &header_info {
          *the_header_info.lock().await = Some(header.clone());
        }
      }

      loop {
        let mut consumed = 0;
        if !recv_bytes.is_empty()
          && let Some((_, header)) = &header_info
        {
          let mut current_header = header.clone();
          let view = lend(&mut recv_bytes);
          let parsed = Self::read_object(
            view.clone(),
            &mut current_header,
            &the_header_info,
            is_closed.clone(),
            items.clone(),
            &previous_object_id,
            &mut fetch_state,
            malformed_track.clone(),
          )
          .await;
          recv_bytes = reclaim(view, parsed.as_ref().map_or(0, |(c, ..)| *c));
          let (c, object_id, new_fetch_ctx) = parsed.map_err(|e| {
            error!("Failed to parse object: {:?}", e);
            RecvDataStreamReadError::ParseError(e)
          })?;

          // if consumed bytes is more than 0, it means we have a valid object
          if c > 0 {
            // The first object may have named the subgroup; every later object on
            // this stream belongs to that same one.
            if let Some(entry) = header_info.as_mut() {
              entry.1 = current_header;
            }
            previous_object_id = object_id;
            if new_fetch_ctx.is_some() {
              fetch_state.prev_ctx = new_fetch_ctx;
            }
            notify.notify_waiters();
          }

          consumed = c;

          trace!(
            "previous_object_id: {:?} object_id: {:?} consumed: {}",
            previous_object_id, &object_id, consumed
          );
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
      if is_closed.load(Ordering::Relaxed) {
        notify.notify_waiters();
        // A FIN in the middle of an Object: the rest of it, and whatever came after,
        // never arrived.
        if !recv_bytes.is_empty() {
          return Err(RecvDataStreamReadError::ParseError(ParseError::Other {
            context: "RecvDataStream::read",
            msg: format!("stream ended {} bytes into an Object", recv_bytes.len()),
          }));
        }
        return Ok(());
      }

      let stream = recv_stream.clone();
      let mut stream = stream.lock().await;

      tokio::select! {
          biased;

          _ = sleep_until(timeout_at) => {
            info!("Timeout while waiting for data");
            is_closed.store(true, Ordering::Relaxed);
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
                is_closed.store(true, Ordering::Relaxed);
              }
              Err(e) => {
                debug!("RecvDataStream::read() Read error: {:?}", e);
                is_closed.store(true, Ordering::Relaxed);
                if matches!(e, TransportReadError::NotConnected) {
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
    is_closed: Arc<AtomicBool>,
    pending_fetches: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
  ) -> Result<Option<(usize, HeaderInfo)>, ParseError> {
    debug!("RecvDataStream::read_header() called");
    let original_remaining = bytes_cursor.remaining();
    if is_fetch {
      match FetchHeader::deserialize(&mut bytes_cursor) {
        Ok(fetch_header) => {
          // A Request ID with no FETCH behind it is not an error: a fill fetch
          // stream is named by the SUBSCRIBE or REQUEST_UPDATE that asked for the
          // fill, which this map knows nothing about.
          let fetch_request = pending_fetches
            .read()
            .await
            .get(&fetch_header.request_id)
            .map(|pending| pending.fetch_request.clone());
          let consumed = original_remaining - bytes_cursor.remaining();

          let header_info = HeaderInfo::Fetch {
            header: fetch_header,
            fetch_request,
          };
          debug!(
            "RecvDataStream::read_header() Parsed FetchHeader: {:?}",
            header_info
          );
          Ok(Some((consumed, header_info)))
        }
        Err(ParseError::NotEnoughBytes { .. }) => {
          Ok(None) // Not enough bytes to parse the header, wait for more data
        }
        Err(e) => {
          is_closed.store(true, Ordering::Relaxed);
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
          is_closed.store(true, Ordering::Relaxed);
          Err(ParseError::ProtocolViolation {
            context: "RecvDataStream::new(SubgroupHeader validation)",
            details: e.to_string(),
          })
        }
      }
    }
  }

  #[allow(clippy::too_many_arguments)]
  async fn read_object(
    mut bytes_cursor: bytes::Bytes,
    header_info: &mut HeaderInfo,
    the_header_info: &Arc<Mutex<Option<HeaderInfo>>>,
    is_closed: Arc<AtomicBool>,
    items: Arc<RwLock<VecDeque<DataStreamItem>>>,
    previous_object_id: &Option<u64>,
    fetch_state: &mut FetchReadState,
    malformed_track: Arc<AtomicBool>,
  ) -> Result<(usize, Option<u64>, Option<FetchObjectContext>), ParseError> {
    if !bytes_cursor.is_empty() {
      let original_remaining = bytes_cursor.remaining();

      // A Subgroup header whose type says the Subgroup ID is the first Object ID
      // carries no Subgroup ID field, so the header alone does not identify the
      // subgroup: the object parsed below is what names it. Left unresolved, every
      // such subgroup in a group is indistinguishable from every other.
      let awaiting_subgroup_id = matches!(
        &header_info,
        HeaderInfo::Subgroup { header, .. }
          if header.subgroup_id.is_none() && header.header_type.subgroup_id_is_first_object_id()
      );

      let parse_result: ObjectParseResult = match header_info {
        HeaderInfo::Fetch { fetch_request, .. } => {
          let group_order = fetch_group_order(fetch_request.as_ref());
          let prev_ctx = fetch_state.prev_ctx;
          FetchObject::deserialize(&mut bytes_cursor, prev_ctx.as_ref(), group_order).and_then(
            |fetch_obj| {
              // A marker moves the prior Location on too, so the context is carried
              // past it as past an Object.
              let new_ctx = Some(fetch_obj.next_context(prev_ctx.as_ref()));
              match fetch_obj {
                FetchObject::Object(payload) => {
                  let validated =
                    fetch_state
                      .subgroup_priorities
                      .validate(&payload)
                      .and_then(|()| {
                        validate_prior_gaps(
                          payload.group_id,
                          payload.object_id,
                          payload.properties.as_deref(),
                        )
                      });
                  if let Err(e) = validated {
                    malformed_track.store(true, Ordering::Relaxed);
                    return Err(e);
                  }
                  // A fetch stream carries no Track Alias. The 0 here is a placeholder
                  // that may well collide with a real subscription's alias, so
                  // callers route these objects by the FETCH_HEADER's Request ID
                  // instead: a FETCH's when `fetch_request` is set, and otherwise
                  // that of the SUBSCRIBE or REQUEST_UPDATE whose fill this stream
                  // delivers, which only the caller can map to a subscription.
                  let object = Object::try_from_fetch(payload, 0)?;
                  Ok((None, new_ctx, Some(DataStreamItem::Object(object))))
                }
                FetchObject::EndOfRange {
                  kind,
                  group_id,
                  object_id,
                } => Ok((
                  None,
                  new_ctx,
                  Some(DataStreamItem::EndOfRange {
                    kind,
                    location: Location::new(group_id, object_id),
                  }),
                )),
              }
            },
          )
        }
        HeaderInfo::Subgroup { header, .. } => {
          let has_properties = header.header_type.has_properties();
          SubgroupObject::deserialize(&mut bytes_cursor, previous_object_id, has_properties)
            .and_then(|subgroup_obj| {
              let object_id = subgroup_obj.object_id;
              if awaiting_subgroup_id {
                header.subgroup_id = Some(object_id);
              }
              let object = Object::try_from_subgroup(
                subgroup_obj,
                header.track_alias,
                header.group_id,
                header.subgroup_id,
                header.publisher_priority,
                DEFAULT_PUBLISHER_PRIORITY,
              )?;
              Ok((Some(object_id), None, Some(DataStreamItem::Object(object))))
            })
        }
      };

      match parse_result {
        Ok((object_id, new_ctx, maybe_item)) => {
          let consumed = original_remaining - bytes_cursor.remaining();
          trace!("consumed: {} Parsed item: {:?}", consumed, maybe_item);
          // Published before the object is enqueued: a reader that takes the object
          // asks for the header to route it, and must not see the unresolved one.
          if awaiting_subgroup_id && consumed > 0 {
            *the_header_info.lock().await = Some(header_info.clone());
          }
          if let Some(item) = maybe_item {
            items.write().await.push_back(item);
          }
          Ok((consumed, object_id, new_ctx))
        }
        Err(ParseError::NotEnoughBytes { .. }) => {
          trace!("Not enough bytes to parse the object, continuing to read...");
          Ok((0, None, None))
        }
        Err(e) => {
          is_closed.store(true, Ordering::Relaxed);
          Err(ParseError::ProtocolViolation {
            context: "RecvDataStream::next_object(parse_result)",
            details: e.to_string(),
          })
        }
      }
    } else {
      debug!("No bytes available to parse an object");
      Ok((0, None, None))
    }
  }

  /// The next Object, passing over a fetch stream's End of Range markers; `None` once
  /// the stream has ended.
  pub async fn next_object(&self) -> (&Self, Option<Object>) {
    loop {
      match self.next_item().await.1 {
        Some(DataStreamItem::Object(object)) => return (self, Some(object)),
        Some(DataStreamItem::EndOfRange { .. }) => continue,
        None => return (self, None),
      }
    }
  }

  /// The next Object or End of Range marker; `None` once the stream has ended.
  pub async fn next_item(&self) -> (&Self, Option<DataStreamItem>) {
    // Start the read task only once
    if self
      .started_read_task
      .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
      .is_ok()
    {
      let recv_stream = self.recv_stream.clone();
      let is_closed = self.is_closed.clone();
      let malformed_track = self.malformed_track.clone();
      let pending_fetches = self.pending_fetches.clone();
      let items = self.items.clone();
      let header_info = self.header_info.clone();
      let notify = self.notify.clone();
      let read_outcome = self.read_outcome.clone();
      let qlog_conn = self.qlog_conn;

      // Spawn the read task that will continuously read from the stream
      // and queue what it parses
      tokio::spawn(async move {
        match Self::read(
          recv_stream,
          is_closed,
          header_info,
          pending_fetches,
          items,
          malformed_track,
          notify.clone(),
          qlog_conn,
        )
        .await
        {
          Ok(_) => {
            read_outcome.store(READ_FINISHED, Ordering::Release);
            debug!("RecvDataStream read task completed successfully")
          }
          Err(e) => {
            read_outcome.store(READ_ABORTED, Ordering::Release);
            error!("RecvDataStream read task encountered an error: {:?}", e);
            notify.notify_waiters();
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

    loop {
      // Registered before the checks below: the read task's notify_waiters keeps no
      // permit, so a wakeup sent between a check and the wait would be lost.
      let notified = self.notify.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();

      // First, check if anything is queued
      {
        let mut items = self.items.write().await;
        if let Some(item) = items.pop_front() {
          return (self, Some(item));
        }
      }

      if self.is_closed.load(Ordering::Relaxed) {
        if self.items.read().await.is_empty() {
          // The read task closes the stream just before it ends; wait for it, so how
          // it ended is known by the time the caller sees the end.
          if self.read_outcome.load(Ordering::Acquire) == READ_RUNNING {
            yield_now().await;
            continue;
          }
          debug!("Stream is closed, returning EOF");
          return (self, None);
        } else {
          debug!("Stream is closed, but has items still");
          yield_now().await
        }
      } else {
        // Wait for notification that a new object may be available or the stream is closed.
        notified.await;
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::common::location::Location;
  use crate::model::common::pair::KeyValuePair;
  use crate::model::common::tuple::{Tuple, TupleField};
  use crate::model::control::constant::GroupOrder;
  use crate::model::control::{fetch::Fetch, subscribe::Subscribe};
  use crate::model::data::constant::SubgroupHeaderType;
  use crate::model::parameter::authorization_token::AuthorizationToken;
  use crate::model::parameter::location_filter::LocationFilter;
  use crate::model::parameter::message_parameter::MessageParameter;
  use crate::model::property::object_property::ObjectProperty;
  use bytes::Bytes;
  use std::error::Error;
  use std::sync::Arc;
  use tokio::sync::Mutex;
  use tokio::time::{Duration, sleep};
  use wtransport::endpoint::IntoConnectOptions;
  use wtransport::{ClientConfig, Connection, Endpoint, Identity, RecvStream, SendStream};

  fn make_fetch_header_and_request() -> (FetchHeader, Fetch) {
    let fetch = Fetch {
      request_id: 161803,
      track_namespace: Tuple::from_utf8_path("un/deux/trois"),
      track_name: TupleField::from_utf8("quatre"),
      parameters: vec![
        MessageParameter::new_location_filter(LocationFilter::AbsoluteRange {
          start: Location::new(12, 5),
          end: Location::new(20, 3),
        }),
        MessageParameter::new_authorization_token(AuthorizationToken::new_use_value(
          0,
          Bytes::from_static(b"test-token"),
        )),
        MessageParameter::new_subscriber_priority(42),
        MessageParameter::new_group_order(GroupOrder::Ascending),
      ],
    };
    (FetchHeader { request_id: 161803 }, fetch)
  }

  fn make_fetch_object() -> crate::model::data::fetch_object::FetchObjectPayload {
    use crate::model::data::constant::ObjectForwardingPreference;
    use crate::model::data::fetch_object::FetchObjectPayload;
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
      payload: Bytes::from_static(
        b"01239gjawkk92837aljwdnjwandjnanwdjnajwndkjawndjkanwdkjnawkjddmi",
      ),
    }
  }

  fn make_object_from_fetch(
    fetch_obj: &crate::model::data::fetch_object::FetchObjectPayload,
  ) -> Object {
    Object::try_from_fetch(fetch_obj.clone(), 0).unwrap()
  }

  #[allow(dead_code)]
  fn make_subgroup_header_and_request() -> (SubgroupHeader, Subscribe) {
    let request_id = 128242;
    let track_namespace = Tuple::from_utf8_path("nein/nein/nein");
    let track_name = TupleField::from_utf8("track_42");
    let start_location = Location {
      group: 81,
      object: 81,
    };
    let subscribe = Subscribe::new_with_location_filter(
      request_id,
      track_namespace,
      track_name,
      LocationFilter::AbsoluteStartGroupEnd {
        start: start_location,
        end_group: 25,
      },
      vec![
        MessageParameter::new_subscriber_priority(31),
        MessageParameter::new_group_order(GroupOrder::Original),
        MessageParameter::new_forward(true),
      ],
    );
    let header_type = SubgroupHeaderType::try_new(0x15).unwrap();
    let track_alias = 999;
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
    (subgroup_header, subscribe)
  }

  #[allow(dead_code)]
  fn make_subgroup_object() -> SubgroupObject {
    let object_id: u64 = 10;
    let properties = Some(vec![
      ObjectProperty::Unknown {
        kvp: KeyValuePair::try_new_varint(0, 10).unwrap(),
      },
      ObjectProperty::Unknown {
        kvp: KeyValuePair::try_new_bytes(1, Bytes::from_static(b"wololoo")).unwrap(),
      },
    ]);
    let object_status = None;
    let payload = Some(Bytes::from_static(b"01239gjawkk92837aldmi"));

    SubgroupObject {
      object_id,
      properties,
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
      DEFAULT_PUBLISHER_PRIORITY,
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
        fetch_request: Some(fetch_req.clone()),
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
        fetch_request: Some(fetch_req.clone()),
        track_alias: 1,
      },
    );
    let mut pending_subscribes = BTreeMap::new();
    // Send only the header, not the object
    let mut _sender = SendDataStream::new(
      Arc::new(Mutex::new(send)),
      HeaderInfo::Fetch {
        header: fetch_header.clone(),
        fetch_request: Some(fetch_req.clone()),
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
  /// Opens a fetch stream, writes one Object on it less its last `cut` bytes, then ends
  /// it with `end`. Returns whether the reader saw the stream aborted once it reported
  /// the end.
  async fn end_fetch_stream(cut: usize, end: impl AsyncFnOnce(&mut TransportSendStream)) -> bool {
    let (send, recv) = setup_stream_pair().await;
    let (fetch_header, fetch_req) = make_fetch_header_and_request();
    let sender = SendDataStream::new(
      Arc::new(Mutex::new(TransportSendStream::WebTransport(send))),
      HeaderInfo::Fetch {
        header: fetch_header,
        fetch_request: Some(fetch_req.clone()),
      },
    )
    .await
    .unwrap();
    let receiver = RecvDataStream::new(
      TransportRecvStream::WebTransport(recv),
      Arc::new(RwLock::new(BTreeMap::new())),
    );

    let bytes = FetchObject::Object(make_fetch_object())
      .serialize(None, fetch_req.group_order())
      .unwrap();
    {
      let mut stream = sender.send_stream.lock().await;
      stream.write_all(&bytes[..bytes.len() - cut]).await.unwrap();
      end(&mut stream).await;
    }

    let mut objects = 0;
    while receiver.next_object().await.1.is_some() {
      objects += 1;
    }
    assert!(objects <= 1);
    receiver.was_aborted()
  }

  /// A fetch stream carrying an Object at {9,10} and an End of Unknown Range marker at
  /// {10,5}, closed with a FIN.
  async fn marked_fetch_stream() -> RecvDataStream {
    let (send, recv) = setup_stream_pair().await;
    let (fetch_header, fetch_req) = make_fetch_header_and_request();
    let order = fetch_req.group_order();
    let sender = SendDataStream::new(
      Arc::new(Mutex::new(TransportSendStream::WebTransport(send))),
      HeaderInfo::Fetch {
        header: fetch_header,
        fetch_request: Some(fetch_req),
      },
    )
    .await
    .unwrap();
    let receiver = RecvDataStream::new(
      TransportRecvStream::WebTransport(recv),
      Arc::new(RwLock::new(BTreeMap::new())),
    );

    let marker = FetchObject::EndOfRange {
      kind: EndOfRangeKind::Unknown,
      group_id: 10,
      object_id: 5,
    };
    let mut stream = sender.send_stream.lock().await;
    for entry in [FetchObject::Object(make_fetch_object()), marker] {
      stream
        .write_all(&entry.serialize(None, order).unwrap())
        .await
        .unwrap();
    }
    stream.finish().await.unwrap();
    receiver
  }

  #[tokio::test]
  async fn next_item_hands_over_end_of_range_markers_in_order() {
    let receiver = marked_fetch_stream().await;
    let mut items = Vec::new();
    while let (_, Some(item)) = receiver.next_item().await {
      items.push(match item {
        DataStreamItem::Object(object) => format!("object {:?}", object.location),
        DataStreamItem::EndOfRange { kind, location } => format!("{kind:?} to {location:?}"),
      });
    }
    let expected = [
      format!("object {:?}", Location::new(9, 10)),
      format!("Unknown to {:?}", Location::new(10, 5)),
    ];
    assert_eq!(items, expected);
  }

  #[tokio::test]
  async fn next_object_passes_over_end_of_range_markers() {
    let receiver = marked_fetch_stream().await;
    let mut locations = Vec::new();
    while let (_, Some(object)) = receiver.next_object().await {
      locations.push(object.location);
    }
    assert_eq!(locations, [Location::new(9, 10)]);
  }

  #[tokio::test]
  async fn objects_after_a_marker_keep_their_ids() {
    let (send, recv) = setup_stream_pair().await;
    let (fetch_header, fetch_req) = make_fetch_header_and_request();
    let mut sender = SendDataStream::new(
      Arc::new(Mutex::new(TransportSendStream::WebTransport(send))),
      HeaderInfo::Fetch {
        header: fetch_header,
        fetch_request: Some(fetch_req),
      },
    )
    .await
    .unwrap();
    let receiver = RecvDataStream::new(
      TransportRecvStream::WebTransport(recv),
      Arc::new(RwLock::new(BTreeMap::new())),
    );

    let at = |group_id, object_id| {
      let mut payload = make_fetch_object();
      payload.group_id = group_id;
      payload.object_id = object_id;
      Object::try_from_fetch(payload, 0).unwrap()
    };
    sender.send_object(&at(9, 10), None).await.unwrap();
    sender
      .send_end_of_range(EndOfRangeKind::Unknown, &Location::new(10, 5))
      .await
      .unwrap();
    // One in the marker's group, right after it, and one in the next group.
    sender.send_object(&at(10, 6), None).await.unwrap();
    sender.send_object(&at(11, 0), None).await.unwrap();
    sender.send_stream.lock().await.finish().await.unwrap();

    let mut locations = Vec::new();
    while let (_, Some(object)) = receiver.next_object().await {
      locations.push(object.location);
    }
    assert_eq!(
      locations,
      [
        Location::new(9, 10),
        Location::new(10, 6),
        Location::new(11, 0)
      ]
    );
  }

  #[tokio::test]
  async fn a_stream_ended_with_its_fin_is_not_aborted() {
    let aborted = end_fetch_stream(0, async |stream: &mut TransportSendStream| {
      stream.finish().await.unwrap();
    })
    .await;
    assert!(!aborted);
  }

  #[tokio::test]
  async fn a_header_split_across_reads_is_waited_for() {
    let (send, recv) = setup_stream_pair().await;
    let (fetch_header, fetch_req) = make_fetch_header_and_request();
    let receiver = RecvDataStream::new(
      TransportRecvStream::WebTransport(recv),
      Arc::new(RwLock::new(BTreeMap::new())),
    );
    let header = fetch_header.serialize().unwrap();
    let object = FetchObject::Object(make_fetch_object())
      .serialize(None, fetch_req.group_order())
      .unwrap();

    // Written from another task, since the reader only starts on the first next_object:
    // the header must reach it in two parts.
    tokio::spawn(async move {
      let mut stream = TransportSendStream::WebTransport(send);
      stream.write_all(&header[..1]).await.unwrap();
      sleep(Duration::from_millis(50)).await;
      stream.write_all(&header[1..]).await.unwrap();
      stream.write_all(&object).await.unwrap();
      stream.finish().await.unwrap();
    });

    let first = tokio::time::timeout(Duration::from_secs(5), receiver.next_object())
      .await
      .expect("the reader gave up on the stream");
    assert!(first.1.is_some());
    assert!(receiver.next_object().await.1.is_none());
    assert!(!receiver.was_aborted());
  }

  #[tokio::test]
  async fn a_fin_in_the_middle_of_an_object_is_aborted() {
    let aborted = end_fetch_stream(1, async |stream: &mut TransportSendStream| {
      stream.finish().await.unwrap();
    })
    .await;
    assert!(aborted);
  }

  #[tokio::test]
  async fn a_reset_stream_is_aborted() {
    let aborted = end_fetch_stream(0, async |stream: &mut TransportSendStream| {
      // Give the Object time to leave before the reset discards what is unsent.
      sleep(Duration::from_millis(50)).await;
      stream.reset(0).unwrap();
    })
    .await;
    assert!(aborted);
  }

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
      Arc::new(Mutex::new(TransportSendStream::WebTransport(send))),
      HeaderInfo::Fetch {
        header: fetch_header,
        fetch_request: Some(fetch_req.clone()),
      },
    )
    .await
    .unwrap();

    let fetch_obj = make_fetch_object();
    let object = make_object_from_fetch(&fetch_obj);

    let pending_fetches = Arc::new(RwLock::new(pending_fetches));

    let receiver = RecvDataStream::new(TransportRecvStream::WebTransport(recv), pending_fetches);

    // Serialize object and send in two parts
    let bytes = FetchObject::Object(fetch_obj.clone())
      .serialize(None, fetch_req.group_order())
      .unwrap();
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

  #[tokio::test]
  async fn objects_spanning_many_reads_arrive_intact() {
    // Large objects span many reads; small ones share one. Each must arrive whole and
    // in order.
    let (send, recv) = setup_stream_pair().await;
    let (subgroup_header, _) = make_subgroup_header_and_request();
    let mut sender = SendDataStream::new(
      Arc::new(Mutex::new(TransportSendStream::WebTransport(send))),
      HeaderInfo::Subgroup {
        header: subgroup_header,
      },
    )
    .await
    .unwrap();
    let receiver = RecvDataStream::new(
      TransportRecvStream::WebTransport(recv),
      Arc::new(RwLock::new(BTreeMap::new())),
    );

    let mut sent = Vec::new();
    let mut previous = None;
    for (i, len) in [60_000usize, 5, 40_000, 3].into_iter().enumerate() {
      let mut subgroup_obj = make_subgroup_object();
      subgroup_obj.object_id = 10 + i as u64;
      subgroup_obj.payload = Some(Bytes::from(
        (0..len)
          .map(|b| ((b * 7 + i) % 251) as u8)
          .collect::<Vec<u8>>(),
      ));
      let object = make_object_from_subgroup(&subgroup_obj, &subgroup_header);
      sender.send_object(&object, previous).await.unwrap();
      previous = Some(subgroup_obj.object_id);
      sent.push(object);
    }

    for object in sent {
      assert_eq!(receiver.next_object().await.1.unwrap(), object);
    }
  }
}
