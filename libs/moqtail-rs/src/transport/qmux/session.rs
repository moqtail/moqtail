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

//! A QMux connection: streams, flow control and the order frames go out in.
//!
//! All state sits behind one mutex that is never held across an `.await`. Two tasks
//! (see `ws.rs`) drive it: a reader that applies incoming records and never waits on
//! the application, and a writer that builds one record at a time. Unsent stream data
//! stays in per-stream queues, not in the socket, so a reset or a priority change still
//! applies to it.

use super::codec::{
  DEFAULT_MAX_RECORD_SIZE, Frame, ProtocolError, STREAM_HEADER_MAX, TransportParams, code,
  decode_record, put_varint, varint_len,
};
use crate::transport::connection::{
  CloseReason, TransportConnectionError, TransportReadError, TransportStats, TransportWriteError,
};
use bytes::{Buf, Bytes};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// Limits this endpoint grants its peer.
#[derive(Debug, Clone)]
pub struct QmuxConfig {
  /// Bidirectional streams the peer may have open at once.
  pub max_streams_bidi: u64,
  /// Unidirectional streams the peer may have open at once.
  pub max_streams_uni: u64,
}

impl Default for QmuxConfig {
  fn default() -> Self {
    Self {
      max_streams_bidi: 100,
      max_streams_uni: 1000,
    }
  }
}

/// Receive window per stream, and for the whole connection.
const STREAM_WINDOW: u64 = 1 << 20;
const CONN_WINDOW: u64 = 16 << 20;
/// Unsent bytes a stream may queue before `write` waits.
const SEND_QUEUE_LIMIT: usize = 256 * 1024;
/// Datagrams held in each direction; the oldest is dropped past this.
const DATAGRAM_QUEUE: usize = 64;
/// Largest record we build, whatever the peer allows.
const MAX_RECORD_CAP: u64 = 1 << 20;
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

static NEXT_STABLE_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Default)]
pub(crate) struct Wakes {
  streams: Vec<Arc<Notify>>,
  all: bool,
  accept: bool,
  open: bool,
  datagram: bool,
  writer: bool,
  ready: bool,
}

pub(crate) struct Shared {
  state: Mutex<State>,
  pub(crate) writer: Notify,
  accept: Notify,
  open: Notify,
  datagram: Notify,
  pub(crate) closed: Notify,
  ready: Notify,
  remote: SocketAddr,
  stable_id: usize,
}

impl Shared {
  pub(crate) fn new(is_client: bool, config: &QmuxConfig, remote: SocketAddr) -> Self {
    Self {
      state: Mutex::new(State::new(is_client, config)),
      writer: Notify::new(),
      accept: Notify::new(),
      open: Notify::new(),
      datagram: Notify::new(),
      closed: Notify::new(),
      ready: Notify::new(),
      remote,
      stable_id: NEXT_STABLE_ID.fetch_add(1, Ordering::Relaxed),
    }
  }

  /// Runs `f` on the state, then wakes whoever its changes concern.
  pub(crate) fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
    let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
    let result = f(&mut state);
    let wakes = std::mem::take(&mut state.wakes);
    if wakes.all {
      for stream in state.streams.values() {
        stream.wake.notify_waiters();
      }
      self.closed.notify_waiters();
    }
    for notify in wakes.streams {
      notify.notify_waiters();
    }
    if wakes.accept || wakes.all {
      self.accept.notify_waiters();
    }
    if wakes.open || wakes.all {
      self.open.notify_waiters();
    }
    if wakes.datagram || wakes.all {
      self.datagram.notify_waiters();
    }
    if wakes.ready || wakes.all {
      self.ready.notify_waiters();
    }
    if wakes.writer || wakes.all {
      self.writer.notify_one();
    }
    result
  }

  /// Waits until `poll` returns a value. `notify` must be one `poll`'s changes wake.
  pub(crate) async fn wait<T>(
    &self,
    notify: &Notify,
    mut poll: impl FnMut(&mut State) -> Option<T>,
  ) -> T {
    loop {
      // Created before polling, so a wake between the poll and the await is not lost.
      let notified = notify.notified();
      if let Some(value) = self.with(&mut poll) {
        return value;
      }
      notified.await;
    }
  }

  pub(crate) fn ready(&self) -> &Notify {
    &self.ready
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fin {
  Open,
  Queued,
  Sent,
}

struct SendHalf {
  queue: VecDeque<Bytes>,
  queued: usize,
  /// Bytes handed to the socket so far.
  offset: u64,
  /// Highest offset the peer lets us send.
  max: u64,
  fin: Fin,
  reset: Option<u64>,
  stopped: Option<u64>,
  priority: i32,
  blocked_at: Option<u64>,
  handle: bool,
}

impl SendHalf {
  fn new(max: u64) -> Self {
    Self {
      queue: VecDeque::new(),
      queued: 0,
      offset: 0,
      max,
      fin: Fin::Open,
      reset: None,
      stopped: None,
      priority: 0,
      blocked_at: None,
      handle: true,
    }
  }

  fn done(&self) -> bool {
    self.reset.is_some() || self.fin == Fin::Sent
  }

  fn has_work(&self) -> bool {
    self.reset.is_none() && (self.queued > 0 || self.fin == Fin::Queued)
  }
}

struct RecvHalf {
  buf: VecDeque<Bytes>,
  /// Next offset expected from the peer.
  received: u64,
  /// Bytes the application has read (or that were discarded for it).
  consumed: u64,
  /// Highest offset we let the peer send.
  max: u64,
  window: u64,
  fin: Option<u64>,
  reset: Option<u64>,
  stopped: bool,
  handle: bool,
}

impl RecvHalf {
  fn new(window: u64) -> Self {
    Self {
      buf: VecDeque::new(),
      received: 0,
      consumed: 0,
      max: window,
      window,
      fin: None,
      reset: None,
      stopped: false,
      handle: true,
    }
  }

  fn done(&self) -> bool {
    self.reset.is_some() || self.stopped || self.fin == Some(self.consumed)
  }
}

struct Stream {
  wake: Arc<Notify>,
  send: Option<SendHalf>,
  recv: Option<RecvHalf>,
}

pub(crate) struct Outgoing {
  pub record: Vec<u8>,
  /// The record carries our CONNECTION_CLOSE: nothing may follow it.
  pub last: bool,
}

pub(crate) struct State {
  is_client: bool,
  local: TransportParams,
  pub(crate) peer: Option<TransportParams>,
  params_sent: bool,

  next_bidi: u64,
  next_uni: u64,
  peer_max_bidi: u64,
  peer_max_uni: u64,
  peer_opened_bidi: u64,
  peer_opened_uni: u64,
  local_max_bidi: u64,
  local_max_uni: u64,
  max_streams_dirty: [bool; 2],

  streams: HashMap<u64, Stream>,
  accept_bidi: VecDeque<u64>,
  accept_uni: VecDeque<u64>,

  send_max_data: u64,
  sent_data: u64,
  data_blocked_at: Option<u64>,
  recv_max_data: u64,
  recv_data: u64,
  consumed_data: u64,
  max_data_dirty: bool,
  credit_dirty: HashSet<u64>,

  control: VecDeque<Frame>,
  datagrams_out: VecDeque<Bytes>,
  datagrams_in: VecDeque<Bytes>,

  pub(crate) close: Option<CloseReason>,
  close_frame: Option<Frame>,
  rr_cursor: u64,

  pub(crate) stats: TransportStats,
  ping: Option<(u64, Instant)>,
  ping_seq: u64,
  last_rx: Instant,
  wakes: Wakes,
}

impl State {
  fn new(is_client: bool, config: &QmuxConfig) -> Self {
    let local = TransportParams {
      max_idle_timeout_ms: IDLE_TIMEOUT.as_millis() as u64,
      initial_max_data: CONN_WINDOW,
      initial_max_stream_data_bidi_local: STREAM_WINDOW,
      initial_max_stream_data_bidi_remote: STREAM_WINDOW,
      initial_max_stream_data_uni: STREAM_WINDOW,
      initial_max_streams_bidi: config.max_streams_bidi,
      initial_max_streams_uni: config.max_streams_uni,
      max_datagram_frame_size: DEFAULT_MAX_RECORD_SIZE,
      max_record_size: DEFAULT_MAX_RECORD_SIZE,
    };
    Self {
      is_client,
      peer: None,
      params_sent: false,
      next_bidi: 0,
      next_uni: 0,
      peer_max_bidi: 0,
      peer_max_uni: 0,
      peer_opened_bidi: 0,
      peer_opened_uni: 0,
      local_max_bidi: local.initial_max_streams_bidi,
      local_max_uni: local.initial_max_streams_uni,
      max_streams_dirty: [false; 2],
      streams: HashMap::new(),
      accept_bidi: VecDeque::new(),
      accept_uni: VecDeque::new(),
      send_max_data: 0,
      sent_data: 0,
      data_blocked_at: None,
      recv_max_data: local.initial_max_data,
      recv_data: 0,
      consumed_data: 0,
      max_data_dirty: false,
      credit_dirty: HashSet::new(),
      control: VecDeque::new(),
      datagrams_out: VecDeque::new(),
      datagrams_in: VecDeque::new(),
      close: None,
      close_frame: None,
      rr_cursor: 0,
      stats: TransportStats::default(),
      ping: None,
      ping_seq: 0,
      last_rx: Instant::now(),
      local,
      wakes: Wakes::default(),
    }
  }

  fn is_local(&self, id: u64) -> bool {
    (id & 1 == 0) == self.is_client
  }

  fn wake_stream(&mut self, id: u64) {
    if let Some(stream) = self.streams.get(&id) {
      self.wakes.streams.push(stream.wake.clone());
    }
  }

  fn queue_control(&mut self, frame: Frame) {
    self.control.push_back(frame);
    self.wakes.writer = true;
  }

  pub(crate) fn closed_error(&self) -> TransportConnectionError {
    match &self.close {
      Some(CloseReason::ByPeer { .. }) => TransportConnectionError::ApplicationClosed,
      Some(reason) => TransportConnectionError::Other(format!("connection closed: {reason:?}")),
      None => TransportConnectionError::Other("connection open".to_string()),
    }
  }

  // ---- connection close -------------------------------------------------------

  fn shut(&mut self, reason: CloseReason, frame: Option<Frame>) {
    if self.close.is_some() {
      return;
    }
    self.close = Some(reason);
    self.close_frame = frame;
    self.control.clear();
    self.datagrams_out.clear();
    for stream in self.streams.values_mut() {
      if let Some(send) = &mut stream.send {
        send.queue.clear();
        send.queued = 0;
      }
    }
    self.wakes.all = true;
  }

  pub(crate) fn close_local(&mut self, code: u64, reason: &[u8]) {
    let frame = Frame::ConnectionClose {
      app: true,
      code,
      frame_type: 0,
      reason: Bytes::copy_from_slice(reason),
    };
    self.shut(CloseReason::Local, Some(frame));
  }

  /// Closes the connection because the peer broke the protocol.
  pub(crate) fn fail(&mut self, error: ProtocolError) {
    let frame = Frame::ConnectionClose {
      app: false,
      code: error.code,
      frame_type: 0,
      reason: Bytes::from(error.reason.clone().into_bytes()),
    };
    self.shut(
      CloseReason::Other(format!("QMux error {:#x}: {}", error.code, error.reason)),
      Some(frame),
    );
  }

  /// The WebSocket ended under us: closed, reset or timed out.
  pub(crate) fn lost(&mut self, reason: CloseReason) {
    self.shut(reason, None);
  }

  // ---- incoming ----------------------------------------------------------------

  /// Applies one received record.
  pub(crate) fn on_record(&mut self, record: Bytes) {
    self.stats.received_bytes += record.len() as u64;
    self.stats.received_packets += 1;
    self.last_rx = Instant::now();
    if self.close.is_some() {
      return;
    }
    if record.len() as u64 > self.local.max_record_size {
      self.fail(ProtocolError::new(
        code::FRAME_ENCODING_ERROR,
        "record larger than max_record_size",
      ));
      return;
    }
    let frames = match decode_record(record) {
      Ok(frames) => frames,
      Err(error) => return self.fail(error),
    };
    for frame in frames {
      if let Err(error) = self.handle(frame) {
        return self.fail(error);
      }
      if self.close.is_some() {
        return;
      }
    }
  }

  pub(crate) fn on_pong(&mut self, payload: &[u8]) {
    self.last_rx = Instant::now();
    if let Some((seq, sent)) = self.ping
      && payload == seq.to_be_bytes()
    {
      self.stats.rtt = Some(sent.elapsed());
      self.ping = None;
    }
  }

  pub(crate) fn touch(&mut self) {
    self.last_rx = Instant::now();
  }

  /// The next keepalive ping's sequence number, or `None` once the peer has been
  /// silent too long (which closes the connection).
  pub(crate) fn keepalive(&mut self) -> Option<u64> {
    if self.close.is_some() {
      return None;
    }
    if self.last_rx.elapsed() > IDLE_TIMEOUT {
      self.lost(CloseReason::IdleTimeout);
      return None;
    }
    self.ping_seq += 1;
    self.ping = Some((self.ping_seq, Instant::now()));
    Some(self.ping_seq)
  }

  fn handle(&mut self, frame: Frame) -> Result<(), ProtocolError> {
    if self.peer.is_none() {
      let Frame::TransportParameters(params) = frame else {
        return Err(ProtocolError::new(
          code::PROTOCOL_VIOLATION,
          "first frame must be the transport parameters",
        ));
      };
      self.send_max_data = params.initial_max_data;
      self.peer_max_bidi = params.initial_max_streams_bidi;
      self.peer_max_uni = params.initial_max_streams_uni;
      self.peer = Some(params);
      self.wakes.ready = true;
      self.wakes.open = true;
      self.wakes.writer = true;
      return Ok(());
    }
    match frame {
      Frame::Padding
      | Frame::DataBlocked(_)
      | Frame::StreamDataBlocked { .. }
      | Frame::StreamsBlocked { .. }
      | Frame::PingResponse(_) => {}
      Frame::TransportParameters(_) => {
        return Err(ProtocolError::new(
          code::PROTOCOL_VIOLATION,
          "transport parameters sent twice",
        ));
      }
      Frame::Stream {
        id,
        offset,
        data,
        fin,
      } => self.on_stream(id, offset, data, fin)?,
      Frame::ResetStream {
        id,
        code,
        final_size,
      } => self.on_reset(id, code, final_size)?,
      Frame::StopSending { id, code } => self.on_stop_sending(id, code)?,
      Frame::MaxData(max) => {
        if max > self.send_max_data {
          self.send_max_data = max;
          self.wakes.writer = true;
        }
      }
      Frame::MaxStreamData { id, max } => {
        self.check_stream(id, true, false)?;
        if let Some(send) = self.streams.get_mut(&id).and_then(|s| s.send.as_mut())
          && max > send.max
        {
          send.max = max;
          self.wakes.writer = true;
        }
      }
      Frame::MaxStreams { bidi, max } => {
        if max > 1 << 60 {
          return Err(ProtocolError::new(
            code::FRAME_ENCODING_ERROR,
            "stream limit above 2^60",
          ));
        }
        let limit = if bidi {
          &mut self.peer_max_bidi
        } else {
          &mut self.peer_max_uni
        };
        if max > *limit {
          *limit = max;
          self.wakes.open = true;
        }
      }
      Frame::ConnectionClose { code, reason, .. } => {
        self.lost(CloseReason::ByPeer {
          code,
          reason: String::from_utf8_lossy(&reason).into_owned(),
        });
      }
      Frame::Datagram(data) => {
        if self.local.max_datagram_frame_size == 0 {
          return Err(ProtocolError::new(
            code::PROTOCOL_VIOLATION,
            "datagram without having offered them",
          ));
        }
        self.datagrams_in.push_back(data);
        if self.datagrams_in.len() > DATAGRAM_QUEUE {
          self.datagrams_in.pop_front();
        }
        self.wakes.datagram = true;
      }
      Frame::PingRequest(seq) => self.queue_control(Frame::PingResponse(seq)),
    }
    Ok(())
  }

  /// Checks a frame's stream id against who may send what, and opens any peer
  /// streams it implies. The stream may since have been closed and forgotten.
  fn check_stream(
    &mut self,
    id: u64,
    needs_send_half: bool,
    needs_recv_half: bool,
  ) -> Result<(), ProtocolError> {
    let bidi = id & 2 == 0;
    let index = id >> 2;
    let state_error = |reason: &str| ProtocolError::new(code::STREAM_STATE_ERROR, reason);
    if self.is_local(id) {
      let opened = if bidi { self.next_bidi } else { self.next_uni };
      if index >= opened {
        return Err(state_error("frame for a stream this endpoint never opened"));
      }
      if !bidi && needs_recv_half {
        return Err(state_error("data on our own unidirectional stream"));
      }
      return Ok(());
    }
    if !bidi && needs_send_half {
      return Err(state_error(
        "send-side frame for the peer's unidirectional stream",
      ));
    }
    let (opened, limit) = if bidi {
      (self.peer_opened_bidi, self.local_max_bidi)
    } else {
      (self.peer_opened_uni, self.local_max_uni)
    };
    if index >= limit {
      return Err(ProtocolError::new(
        code::STREAM_LIMIT_ERROR,
        "peer opened more streams than allowed",
      ));
    }
    // Opening a stream opens every lower-numbered one of its type too.
    for i in opened..=index {
      let new_id = (i << 2) | (id & 3);
      let peer = self.peer.as_ref().expect("checked before any stream frame");
      let stream = Stream {
        wake: Arc::new(Notify::new()),
        send: bidi.then(|| SendHalf::new(peer.initial_max_stream_data_bidi_local)),
        recv: Some(RecvHalf::new(if bidi {
          self.local.initial_max_stream_data_bidi_remote
        } else {
          self.local.initial_max_stream_data_uni
        })),
      };
      self.streams.insert(new_id, stream);
      if bidi {
        self.accept_bidi.push_back(new_id);
      } else {
        self.accept_uni.push_back(new_id);
      }
      self.wakes.accept = true;
    }
    if index >= opened {
      if bidi {
        self.peer_opened_bidi = index + 1;
      } else {
        self.peer_opened_uni = index + 1;
      }
    }
    Ok(())
  }

  /// Counts bytes against the connection window and hands the credit straight back:
  /// used for data nobody will read.
  fn discard(&mut self, len: u64) -> Result<(), ProtocolError> {
    if self.recv_data + len > self.recv_max_data {
      return Err(ProtocolError::new(
        code::FLOW_CONTROL_ERROR,
        "peer exceeded the connection window",
      ));
    }
    self.recv_data += len;
    self.consume_conn(len);
    Ok(())
  }

  fn consume_conn(&mut self, len: u64) {
    self.consumed_data += len;
    if self.recv_max_data - self.consumed_data < CONN_WINDOW / 2 {
      self.recv_max_data = self.consumed_data + CONN_WINDOW;
      self.max_data_dirty = true;
      self.wakes.writer = true;
    }
  }

  fn on_stream(
    &mut self,
    id: u64,
    offset: u64,
    data: Bytes,
    fin: bool,
  ) -> Result<(), ProtocolError> {
    self.check_stream(id, false, true)?;
    let len = data.len() as u64;
    let end = offset + len;
    let conn_room = self.recv_max_data - self.recv_data;
    let Some(recv) = self.streams.get_mut(&id).and_then(|s| s.recv.as_mut()) else {
      // Already closed and forgotten.
      return self.discard(len);
    };
    if recv.reset.is_some() {
      return self.discard(len);
    }
    if offset != recv.received {
      return Err(ProtocolError::new(
        code::PROTOCOL_VIOLATION,
        "stream data out of order",
      ));
    }
    if let Some(final_size) = recv.fin
      && (end > final_size || (fin && end != final_size))
    {
      return Err(ProtocolError::new(
        code::FINAL_SIZE_ERROR,
        "stream data past its final size",
      ));
    }
    if end > recv.max {
      return Err(ProtocolError::new(
        code::FLOW_CONTROL_ERROR,
        "peer exceeded the stream window",
      ));
    }
    if len > conn_room {
      return Err(ProtocolError::new(
        code::FLOW_CONTROL_ERROR,
        "peer exceeded the connection window",
      ));
    }
    recv.received = end;
    if fin {
      recv.fin = Some(end);
    }
    let stopped = recv.stopped;
    if stopped {
      recv.consumed = end;
    } else if len > 0 {
      recv.buf.push_back(data);
    }
    self.recv_data += len;
    if stopped {
      self.consume_conn(len);
    }
    self.wake_stream(id);
    self.maybe_remove(id);
    Ok(())
  }

  fn on_reset(&mut self, id: u64, code: u64, final_size: u64) -> Result<(), ProtocolError> {
    self.check_stream(id, false, true)?;
    let Some(recv) = self.streams.get_mut(&id).and_then(|s| s.recv.as_mut()) else {
      return Ok(());
    };
    if recv.reset.is_some() {
      return Ok(());
    }
    if final_size < recv.received || recv.fin.is_some_and(|f| f != final_size) {
      return Err(ProtocolError::new(
        code::FINAL_SIZE_ERROR,
        "reset with a different final size",
      ));
    }
    if final_size > recv.max {
      return Err(ProtocolError::new(
        code::FLOW_CONTROL_ERROR,
        "reset past the stream window",
      ));
    }
    let new_bytes = final_size - recv.received;
    let unread = final_size - recv.consumed;
    recv.received = final_size;
    recv.consumed = final_size;
    recv.buf.clear();
    recv.reset = Some(code);
    if self.recv_data + new_bytes > self.recv_max_data {
      return Err(ProtocolError::new(
        code::FLOW_CONTROL_ERROR,
        "reset past the connection window",
      ));
    }
    self.recv_data += new_bytes;
    self.consume_conn(unread);
    self.wake_stream(id);
    self.maybe_remove(id);
    Ok(())
  }

  fn on_stop_sending(&mut self, id: u64, code: u64) -> Result<(), ProtocolError> {
    self.check_stream(id, true, false)?;
    let Some(send) = self.streams.get_mut(&id).and_then(|s| s.send.as_mut()) else {
      return Ok(());
    };
    send.stopped = Some(code);
    // The peer won't read any more, so stop sending with its code, as QUIC does.
    if !send.done() {
      self.reset_send(id, code);
    }
    self.wake_stream(id);
    Ok(())
  }

  // ---- stream lifecycle ----------------------------------------------------------

  fn reset_send(&mut self, id: u64, code: u64) {
    let Some(send) = self.streams.get_mut(&id).and_then(|s| s.send.as_mut()) else {
      return;
    };
    send.queue.clear();
    send.queued = 0;
    send.reset = Some(code);
    let final_size = send.offset;
    self.queue_control(Frame::ResetStream {
      id,
      code,
      final_size,
    });
    self.wake_stream(id);
    self.maybe_remove(id);
  }

  /// Forgets a stream once both halves are finished and no handle refers to it, and
  /// returns the stream credit to the peer if it opened the stream.
  fn maybe_remove(&mut self, id: u64) {
    let Some(stream) = self.streams.get(&id) else {
      return;
    };
    let send_gone = stream.send.as_ref().is_none_or(|s| !s.handle && s.done());
    let recv_gone = stream.recv.as_ref().is_none_or(|r| !r.handle && r.done());
    if !(send_gone && recv_gone) {
      return;
    }
    self.streams.remove(&id);
    self.credit_dirty.remove(&id);
    if !self.is_local(id) {
      let bidi = id & 2 == 0;
      if bidi {
        self.local_max_bidi += 1;
      } else {
        self.local_max_uni += 1;
      }
      self.max_streams_dirty[usize::from(!bidi)] = true;
      self.wakes.writer = true;
    }
  }

  fn open(&mut self, bidi: bool) -> Option<Result<(u64, Arc<Notify>), TransportConnectionError>> {
    if self.close.is_some() {
      return Some(Err(self.closed_error()));
    }
    let peer = self.peer.as_ref()?;
    let (next, limit) = if bidi {
      (self.next_bidi, self.peer_max_bidi)
    } else {
      (self.next_uni, self.peer_max_uni)
    };
    if next >= limit {
      return None;
    }
    let id = (next << 2) | u64::from(!self.is_client) | if bidi { 0 } else { 2 };
    let stream = Stream {
      wake: Arc::new(Notify::new()),
      send: Some(SendHalf::new(if bidi {
        peer.initial_max_stream_data_bidi_remote
      } else {
        peer.initial_max_stream_data_uni
      })),
      recv: bidi.then(|| RecvHalf::new(self.local.initial_max_stream_data_bidi_local)),
    };
    let wake = stream.wake.clone();
    self.streams.insert(id, stream);
    if bidi {
      self.next_bidi += 1;
    } else {
      self.next_uni += 1;
    }
    Some(Ok((id, wake)))
  }

  fn accept(&mut self, bidi: bool) -> Option<Result<(u64, Arc<Notify>), TransportConnectionError>> {
    let queue = if bidi {
      &mut self.accept_bidi
    } else {
      &mut self.accept_uni
    };
    if let Some(id) = queue.pop_front() {
      let wake = self.streams.get(&id).map(|s| s.wake.clone())?;
      return Some(Ok((id, wake)));
    }
    self.close.is_some().then(|| Err(self.closed_error()))
  }

  // ---- send side -------------------------------------------------------------------

  fn send_half(&mut self, id: u64) -> Result<&mut SendHalf, TransportWriteError> {
    if self.close.is_some() {
      return Err(TransportWriteError::Other("connection closed".to_string()));
    }
    let send = self
      .streams
      .get_mut(&id)
      .and_then(|s| s.send.as_mut())
      .ok_or(TransportWriteError::ClosedOrStopped)?;
    if let Some(code) = send.stopped {
      return Err(TransportWriteError::Stopped(code));
    }
    if send.reset.is_some() {
      return Err(TransportWriteError::ClosedOrStopped);
    }
    Ok(send)
  }

  /// Queues as many chunks as fit; returns how many were taken.
  fn push(&mut self, id: u64, chunks: &mut [Bytes]) -> Result<usize, TransportWriteError> {
    let send = self.send_half(id)?;
    if send.fin != Fin::Open {
      return Err(TransportWriteError::ClosedOrStopped);
    }
    let mut taken = 0;
    for chunk in chunks.iter_mut() {
      if send.queued >= SEND_QUEUE_LIMIT {
        break;
      }
      let chunk = std::mem::take(chunk);
      if !chunk.is_empty() {
        send.queued += chunk.len();
        send.queue.push_back(chunk);
      }
      taken += 1;
    }
    self.wakes.writer = true;
    Ok(taken)
  }

  fn finish(&mut self, id: u64) -> Result<(), TransportWriteError> {
    let send = self.send_half(id)?;
    if send.fin == Fin::Open {
      send.fin = Fin::Queued;
      self.wakes.writer = true;
    }
    Ok(())
  }

  fn flushed(&mut self, id: u64) -> Option<Result<(), TransportWriteError>> {
    match self.send_half(id) {
      Err(e) => Some(Err(e)),
      Ok(send) if send.queued == 0 && send.fin != Fin::Queued => Some(Ok(())),
      Ok(_) => None,
    }
  }

  fn reset(&mut self, id: u64, code: u64) -> Result<(), TransportWriteError> {
    let send = self.send_half(id)?;
    if send.fin == Fin::Sent {
      return Err(TransportWriteError::ClosedOrStopped);
    }
    self.reset_send(id, code);
    Ok(())
  }

  fn stopped(&mut self, id: u64) -> Option<Option<u64>> {
    if self.close.is_some() {
      return Some(None);
    }
    let Some(send) = self.streams.get(&id).and_then(|s| s.send.as_ref()) else {
      return Some(None);
    };
    if let Some(code) = send.stopped {
      return Some(Some(code));
    }
    send.done().then_some(None)
  }

  fn set_priority(&mut self, id: u64, priority: i32) {
    if let Some(send) = self.streams.get_mut(&id).and_then(|s| s.send.as_mut()) {
      send.priority = priority;
    }
  }

  fn drop_send(&mut self, id: u64) {
    if let Some(send) = self.streams.get_mut(&id).and_then(|s| s.send.as_mut()) {
      send.handle = false;
      // Dropping a stream without finishing it finishes it, as with quinn.
      if send.fin == Fin::Open && send.reset.is_none() {
        send.fin = Fin::Queued;
        self.wakes.writer = true;
      }
    }
    self.maybe_remove(id);
  }

  // ---- receive side ------------------------------------------------------------------

  fn read(&mut self, id: u64, buf: &mut [u8]) -> Option<Result<Option<usize>, TransportReadError>> {
    let Some(recv) = self.streams.get_mut(&id).and_then(|s| s.recv.as_mut()) else {
      return Some(Err(TransportReadError::NotConnected));
    };
    if let Some(code) = recv.reset {
      return Some(Err(TransportReadError::Reset(code)));
    }
    if let Some(front) = recv.buf.front_mut() {
      let n = front.len().min(buf.len());
      buf[..n].copy_from_slice(&front[..n]);
      front.advance(n);
      if front.is_empty() {
        recv.buf.pop_front();
      }
      recv.consumed += n as u64;
      if recv.fin.is_none() && recv.max - recv.consumed < recv.window / 2 {
        recv.max = recv.consumed + recv.window;
        self.credit_dirty.insert(id);
        self.wakes.writer = true;
      }
      self.consume_conn(n as u64);
      return Some(Ok(Some(n)));
    }
    if recv.fin == Some(recv.consumed) {
      return Some(Ok(None));
    }
    if self.close.is_some() {
      return Some(Err(TransportReadError::NotConnected));
    }
    None
  }

  fn stop(&mut self, id: u64, code: u64) {
    let Some(recv) = self.streams.get_mut(&id).and_then(|s| s.recv.as_mut()) else {
      return;
    };
    if recv.done() {
      return;
    }
    recv.stopped = true;
    let unread = recv.received - recv.consumed;
    recv.consumed = recv.received;
    recv.buf.clear();
    self.consume_conn(unread);
    self.queue_control(Frame::StopSending { id, code });
  }

  fn drop_recv(&mut self, id: u64) {
    if let Some(recv) = self.streams.get_mut(&id).and_then(|s| s.recv.as_mut()) {
      recv.handle = false;
      // Dropping a stream unread tells the peer to stop, with code 0, as with quinn.
      if !recv.done() {
        self.stop(id, 0);
      }
    }
    self.maybe_remove(id);
  }

  // ---- datagrams ---------------------------------------------------------------------

  fn send_datagram(&mut self, payload: Bytes) -> Result<(), TransportConnectionError> {
    if self.close.is_some() {
      return Err(self.closed_error());
    }
    let peer = self
      .peer
      .as_ref()
      .ok_or_else(|| TransportConnectionError::Other("handshake not finished".to_string()))?;
    let frame_len = 1 + varint_len(payload.len() as u64) as u64 + payload.len() as u64;
    if peer.max_datagram_frame_size == 0 {
      return Err(TransportConnectionError::Other(
        "peer does not accept datagrams".to_string(),
      ));
    }
    if frame_len > peer.max_datagram_frame_size.min(peer.max_record_size) {
      return Err(TransportConnectionError::Other(
        "datagram too large".to_string(),
      ));
    }
    self.datagrams_out.push_back(payload);
    if self.datagrams_out.len() > DATAGRAM_QUEUE {
      self.datagrams_out.pop_front();
    }
    self.wakes.writer = true;
    Ok(())
  }

  fn receive_datagram(&mut self) -> Option<Result<Bytes, TransportConnectionError>> {
    if let Some(datagram) = self.datagrams_in.pop_front() {
      return Some(Ok(datagram));
    }
    self.close.is_some().then(|| Err(self.closed_error()))
  }

  // ---- outgoing ----------------------------------------------------------------------

  /// Builds the next record to send, or `None` when there is nothing to send.
  pub(crate) fn build_record(&mut self) -> Option<Outgoing> {
    let max = self.peer.as_ref().map_or(DEFAULT_MAX_RECORD_SIZE, |p| {
      p.max_record_size.min(MAX_RECORD_CAP)
    }) as usize;
    let mut out = Vec::new();
    if !self.params_sent {
      Frame::TransportParameters(self.local.clone()).encode(&mut out);
      self.params_sent = true;
    }
    if let Some(frame) = self.close_frame.take() {
      frame.encode(&mut out);
      return Some(Outgoing {
        record: out,
        last: true,
      });
    }
    if self.close.is_some() || self.peer.is_none() {
      return (!out.is_empty()).then_some(Outgoing {
        record: out,
        last: false,
      });
    }

    self.queue_credit();
    while let Some(frame) = self.control.front() {
      if !out.is_empty() && out.len() + frame.encoded_len() > max {
        break;
      }
      self
        .control
        .pop_front()
        .expect("front exists")
        .encode(&mut out);
    }
    while let Some(datagram) = self.datagrams_out.front() {
      if out.len() + STREAM_HEADER_MAX + datagram.len() > max {
        break;
      }
      let datagram = self.datagrams_out.pop_front().expect("front exists");
      Frame::Datagram(datagram).encode(&mut out);
    }
    while out.len() + STREAM_HEADER_MAX < max {
      let Some(id) = self.next_sender() else {
        break;
      };
      self.write_stream_frame(id, max - out.len() - STREAM_HEADER_MAX, &mut out);
    }
    (!out.is_empty()).then_some(Outgoing {
      record: out,
      last: false,
    })
  }

  fn queue_credit(&mut self) {
    if std::mem::take(&mut self.max_data_dirty) {
      self.control.push_back(Frame::MaxData(self.recv_max_data));
    }
    for id in std::mem::take(&mut self.credit_dirty) {
      if let Some(recv) = self.streams.get(&id).and_then(|s| s.recv.as_ref())
        && !recv.done()
      {
        self
          .control
          .push_back(Frame::MaxStreamData { id, max: recv.max });
      }
    }
    for (index, bidi) in [(0, true), (1, false)] {
      if std::mem::take(&mut self.max_streams_dirty[index]) {
        let max = if bidi {
          self.local_max_bidi
        } else {
          self.local_max_uni
        };
        self.control.push_back(Frame::MaxStreams { bidi, max });
      }
    }
  }

  /// The stream whose data goes next: highest priority first, then round-robin by id.
  /// Also signals the peer once per limit when credit runs out.
  fn next_sender(&mut self) -> Option<u64> {
    let conn_credit = self.send_max_data - self.sent_data;
    let cursor = self.rr_cursor;
    let mut best: Option<(i32, bool, u64)> = None;
    let mut blocked = Vec::new();
    let mut conn_blocked = false;
    // shortcut: scans every stream per frame; index ready streams by priority if
    // connections with thousands of streams show up in profiles.
    for (&id, stream) in &mut self.streams {
      let Some(send) = &mut stream.send else {
        continue;
      };
      if !send.has_work() {
        continue;
      }
      let fin_only = send.queued == 0;
      let stream_credit = send.max > send.offset;
      if !fin_only && !(stream_credit && conn_credit > 0) {
        if !stream_credit && send.blocked_at != Some(send.max) {
          send.blocked_at = Some(send.max);
          blocked.push((id, send.max));
        }
        conn_blocked |= conn_credit == 0;
        continue;
      }
      // Ids after the cursor come first, so equal priorities take turns.
      let key = (send.priority, id > cursor, u64::MAX - id);
      if best.is_none_or(|b| key > b) {
        best = Some(key);
      }
    }
    for (id, limit) in blocked {
      self
        .control
        .push_back(Frame::StreamDataBlocked { id, limit });
    }
    if conn_blocked && self.data_blocked_at != Some(self.send_max_data) {
      self.data_blocked_at = Some(self.send_max_data);
      self
        .control
        .push_back(Frame::DataBlocked(self.send_max_data));
    }
    best.map(|(_, _, inverted)| u64::MAX - inverted)
  }

  /// Writes one STREAM frame for `id`, copying straight from its queue into `out`.
  fn write_stream_frame(&mut self, id: u64, room: usize, out: &mut Vec<u8>) {
    let conn_credit = self.send_max_data - self.sent_data;
    let Some(send) = self.streams.get_mut(&id).and_then(|s| s.send.as_mut()) else {
      return;
    };
    let n = (send.queued as u64)
      .min(send.max - send.offset)
      .min(conn_credit)
      .min(room as u64) as usize;
    let fin = send.fin == Fin::Queued && n == send.queued;
    out.push(0x08 | 0x04 | 0x02 | u8::from(fin));
    put_varint(out, id);
    put_varint(out, send.offset);
    put_varint(out, n as u64);
    let mut left = n;
    while left > 0 {
      let front = send.queue.front_mut().expect("queued bytes exist");
      let take = front.len().min(left);
      out.extend_from_slice(&front[..take]);
      front.advance(take);
      if front.is_empty() {
        send.queue.pop_front();
      }
      left -= take;
    }
    send.offset += n as u64;
    send.queued -= n;
    if fin {
      send.fin = Fin::Sent;
    }
    self.sent_data += n as u64;
    self.rr_cursor = id;
    self.wake_stream(id);
    if fin {
      self.maybe_remove(id);
    }
  }
}

// ---- public handles ------------------------------------------------------------------

/// What sessions and streams hold. When the last one goes, the connection closes with
/// code 0, as a quinn connection does once every handle to it is dropped; the reader
/// and writer tasks hold the state directly, so they don't keep it open.
pub(crate) struct Handle(pub(crate) Arc<Shared>);

impl std::ops::Deref for Handle {
  type Target = Shared;
  fn deref(&self) -> &Shared {
    &self.0
  }
}

impl Drop for Handle {
  fn drop(&mut self) {
    self.0.with(|s| s.close_local(0, b""));
  }
}

/// A QMux connection: QUIC-style streams and datagrams over one WebSocket.
#[derive(Clone)]
pub struct QmuxSession {
  pub(crate) shared: Arc<Handle>,
}

impl std::fmt::Debug for QmuxSession {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("QmuxSession")
      .field("remote", &self.shared.remote)
      .field("stable_id", &self.shared.stable_id)
      .finish()
  }
}

impl QmuxSession {
  pub fn remote_address(&self) -> SocketAddr {
    self.shared.remote
  }

  pub fn stable_id(&self) -> usize {
    self.shared.stable_id
  }

  pub fn transport_stats(&self) -> TransportStats {
    self.shared.with(|s| s.stats)
  }

  pub fn close_reason(&self) -> Option<CloseReason> {
    self.shared.with(|s| s.close.clone())
  }

  pub fn close(&self, code: u32, reason: &[u8]) {
    self.shared.with(|s| s.close_local(u64::from(code), reason));
  }

  pub async fn closed(&self) {
    self
      .shared
      .wait(&self.shared.closed, |s| s.close.is_some().then_some(()))
      .await;
  }

  fn send_stream(&self, id: u64, wake: Arc<Notify>) -> QmuxSendStream {
    QmuxSendStream {
      shared: self.shared.clone(),
      id,
      wake,
    }
  }

  fn recv_stream(&self, id: u64, wake: Arc<Notify>) -> QmuxRecvStream {
    QmuxRecvStream {
      shared: self.shared.clone(),
      id,
      wake,
    }
  }

  pub async fn open_bi(
    &self,
  ) -> Result<(QmuxSendStream, QmuxRecvStream), TransportConnectionError> {
    let (id, wake) = self
      .shared
      .wait(&self.shared.open, |s| s.open(true))
      .await?;
    Ok((
      self.send_stream(id, wake.clone()),
      self.recv_stream(id, wake),
    ))
  }

  pub async fn open_uni(&self) -> Result<QmuxSendStream, TransportConnectionError> {
    let (id, wake) = self
      .shared
      .wait(&self.shared.open, |s| s.open(false))
      .await?;
    Ok(self.send_stream(id, wake))
  }

  pub async fn accept_bi(
    &self,
  ) -> Result<(QmuxSendStream, QmuxRecvStream), TransportConnectionError> {
    let (id, wake) = self
      .shared
      .wait(&self.shared.accept, |s| s.accept(true))
      .await?;
    Ok((
      self.send_stream(id, wake.clone()),
      self.recv_stream(id, wake),
    ))
  }

  pub async fn accept_uni(&self) -> Result<QmuxRecvStream, TransportConnectionError> {
    let (id, wake) = self
      .shared
      .wait(&self.shared.accept, |s| s.accept(false))
      .await?;
    Ok(self.recv_stream(id, wake))
  }

  pub fn send_datagram(&self, payload: Bytes) -> Result<(), TransportConnectionError> {
    self.shared.with(|s| s.send_datagram(payload))
  }

  pub async fn receive_datagram(&self) -> Result<Bytes, TransportConnectionError> {
    self
      .shared
      .wait(&self.shared.datagram, |s| s.receive_datagram())
      .await
  }
}

pub struct QmuxSendStream {
  shared: Arc<Handle>,
  id: u64,
  wake: Arc<Notify>,
}

impl std::fmt::Debug for QmuxSendStream {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("QmuxSendStream")
      .field("id", &self.id)
      .finish()
  }
}

impl QmuxSendStream {
  pub fn id(&self) -> u64 {
    self.id
  }

  /// Queues the chunks in order without copying them; waits while the stream's queue
  /// is full.
  pub async fn write_all_chunks(
    &mut self,
    chunks: &mut [Bytes],
  ) -> Result<(), TransportWriteError> {
    let mut done = 0;
    while done < chunks.len() {
      let id = self.id;
      let rest = &mut chunks[done..];
      done += self
        .shared
        .wait(&self.wake, |s| match s.push(id, rest) {
          Ok(0) => None,
          other => Some(other),
        })
        .await?;
    }
    Ok(())
  }

  pub async fn write_all(&mut self, buf: &[u8]) -> Result<(), TransportWriteError> {
    self
      .write_all_chunks(&mut [Bytes::copy_from_slice(buf)])
      .await
  }

  /// Ends the stream after the queued data. Returns at once.
  pub fn finish(&mut self) -> Result<(), TransportWriteError> {
    self.shared.with(|s| s.finish(self.id))
  }

  /// Waits until every queued byte has been handed to the socket.
  pub async fn flush(&mut self) -> Result<(), TransportWriteError> {
    let id = self.id;
    self.shared.wait(&self.wake, |s| s.flushed(id)).await
  }

  pub fn set_priority(&self, priority: i32) {
    self.shared.with(|s| s.set_priority(self.id, priority));
  }

  /// Drops the unsent bytes and resets the stream with `code`.
  pub fn reset(&mut self, code: u64) -> Result<(), TransportWriteError> {
    self.shared.with(|s| s.reset(self.id, code))
  }

  /// The peer's STOP_SENDING code, or `None` once it can no longer come.
  pub async fn stopped(&mut self) -> Option<u64> {
    let id = self.id;
    self.shared.wait(&self.wake, |s| s.stopped(id)).await
  }
}

impl Drop for QmuxSendStream {
  fn drop(&mut self) {
    self.shared.with(|s| s.drop_send(self.id));
  }
}

pub struct QmuxRecvStream {
  shared: Arc<Handle>,
  id: u64,
  wake: Arc<Notify>,
}

impl std::fmt::Debug for QmuxRecvStream {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("QmuxRecvStream")
      .field("id", &self.id)
      .finish()
  }
}

impl QmuxRecvStream {
  pub fn id(&self) -> u64 {
    self.id
  }

  /// `Ok(None)` at the end of the stream.
  pub async fn read(&mut self, buf: &mut [u8]) -> Result<Option<usize>, TransportReadError> {
    if buf.is_empty() {
      return Ok(Some(0));
    }
    let id = self.id;
    self.shared.wait(&self.wake, |s| s.read(id, buf)).await
  }

  /// Asks the peer to stop sending, with `code`.
  pub fn stop(self, code: u64) {
    self.shared.with(|s| s.stop(self.id, code));
  }
}

impl Drop for QmuxRecvStream {
  fn drop(&mut self) {
    self.shared.with(|s| s.drop_recv(self.id));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Two states wired back to back: records move by hand, so tests see exactly what
  /// each side sends.
  struct Pair {
    client: State,
    server: State,
  }

  impl Pair {
    fn new() -> Self {
      let mut pair = Self {
        client: State::new(true, &QmuxConfig::default()),
        server: State::new(false, &QmuxConfig::default()),
      };
      pair.pump();
      assert!(pair.client.peer.is_some() && pair.server.peer.is_some());
      pair
    }

    /// Moves records both ways until neither side has anything to send.
    fn pump(&mut self) {
      loop {
        let mut moved = false;
        while let Some(out) = self.client.build_record() {
          self.server.on_record(Bytes::from(out.record));
          moved = true;
        }
        while let Some(out) = self.server.build_record() {
          self.client.on_record(Bytes::from(out.record));
          moved = true;
        }
        if !moved {
          break;
        }
      }
    }
  }

  fn read_all(state: &mut State, id: u64) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    while let Some(Ok(Some(n))) = state.read(id, &mut buf) {
      data.extend_from_slice(&buf[..n]);
    }
    data
  }

  #[test]
  fn stream_data_and_fin_arrive_in_order() {
    let mut p = Pair::new();
    let (id, _) = p.client.open(false).unwrap().unwrap();
    p.client
      .push(
        id,
        &mut [Bytes::from_static(b"hello "), Bytes::from_static(b"world")],
      )
      .unwrap();
    p.client.finish(id).unwrap();
    p.pump();
    let (accepted, _) = p.server.accept(false).unwrap().unwrap();
    assert_eq!(accepted, id);
    assert_eq!(read_all(&mut p.server, id), b"hello world");
    assert!(matches!(p.server.read(id, &mut [0; 8]), Some(Ok(None))));
  }

  #[test]
  fn a_slow_reader_blocks_only_its_stream_and_credit_resumes_it() {
    let mut p = Pair::new();
    let (slow, _) = p.client.open(false).unwrap().unwrap();
    let (fast, _) = p.client.open(false).unwrap().unwrap();
    let big = Bytes::from(vec![7u8; 3 * STREAM_WINDOW as usize / 2]);
    let mut chunks = [big.clone()];
    p.client.push(slow, &mut chunks).unwrap();
    p.client
      .push(fast, &mut [Bytes::from_static(b"ping")])
      .unwrap();
    p.pump();

    // The slow stream stopped at its window; the other stream still got through.
    let sent = p.client.streams[&slow].send.as_ref().unwrap().offset;
    assert_eq!(sent, STREAM_WINDOW);
    assert_eq!(read_all(&mut p.server, fast), b"ping");

    // Reading the slow stream returns credit and the rest flows.
    let first = read_all(&mut p.server, slow).len();
    p.pump();
    let second = read_all(&mut p.server, slow).len();
    assert_eq!(first + second, big.len());
  }

  #[test]
  fn sending_past_the_window_is_a_flow_control_error() {
    let mut p = Pair::new();
    let (id, _) = p.client.open(false).unwrap().unwrap();
    // Forge a frame the honest sender would never build.
    let mut record = Vec::new();
    Frame::Stream {
      id,
      offset: 0,
      data: Bytes::from(vec![0u8; STREAM_WINDOW as usize + 1]),
      fin: false,
    }
    .encode(&mut record);
    p.server.local.max_record_size = u64::MAX;
    p.server.on_record(Bytes::from(record));
    assert!(
      matches!(&p.server.close, Some(CloseReason::Other(r)) if r.contains("0x3")),
      "{:?}",
      p.server.close
    );
  }

  #[test]
  fn reset_drops_queued_bytes_and_carries_its_code() {
    let mut p = Pair::new();
    let (id, _) = p.client.open(false).unwrap().unwrap();
    p.client
      .push(id, &mut [Bytes::from_static(b"sent")])
      .unwrap();
    p.pump();
    p.client
      .push(id, &mut [Bytes::from_static(b"never sent")])
      .unwrap();
    p.client.reset(id, 0x2).unwrap();
    p.pump();
    assert_eq!(p.client.sent_data, 4, "queued bytes were dropped, not sent");
    assert!(matches!(
      p.server.read(id, &mut [0; 8]),
      Some(Err(TransportReadError::Reset(0x2)))
    ));
  }

  #[test]
  fn stop_sending_resets_the_peer_with_the_same_code() {
    let mut p = Pair::new();
    let (id, _) = p.client.open(false).unwrap().unwrap();
    p.client.push(id, &mut [Bytes::from_static(b"x")]).unwrap();
    p.pump();
    p.server.stop(id, 0x10);
    p.pump();
    assert_eq!(p.client.stopped(id), Some(Some(0x10)));
    assert!(matches!(
      p.client.push(id, &mut [Bytes::from_static(b"y")]),
      Err(TransportWriteError::Stopped(0x10))
    ));
  }

  #[test]
  fn higher_priority_streams_go_first() {
    let mut p = Pair::new();
    let (low, _) = p.client.open(false).unwrap().unwrap();
    let (high, _) = p.client.open(false).unwrap().unwrap();
    p.client.set_priority(high, 10);
    p.client
      .push(low, &mut [Bytes::from(vec![1u8; 20_000])])
      .unwrap();
    p.client
      .push(high, &mut [Bytes::from(vec![2u8; 20_000])])
      .unwrap();
    // One record holds less than either stream's data, so only the winner fits.
    let record = p.client.build_record().unwrap().record;
    let frames = decode_record(Bytes::from(record)).unwrap();
    assert!(
      frames
        .iter()
        .all(|f| !matches!(f, Frame::Stream { id, .. } if *id == low))
    );
    assert!(
      frames
        .iter()
        .any(|f| matches!(f, Frame::Stream { id, .. } if *id == high))
    );
  }

  #[test]
  fn closed_peer_streams_return_stream_credit() {
    let mut p = Pair::new();
    let before = p.server.local_max_uni;
    let (id, _) = p.client.open(false).unwrap().unwrap();
    p.client.finish(id).unwrap();
    p.client.drop_send(id);
    p.pump();
    let (accepted, _) = p.server.accept(false).unwrap().unwrap();
    assert!(matches!(
      p.server.read(accepted, &mut [0; 1]),
      Some(Ok(None))
    ));
    p.server.drop_recv(accepted);
    p.pump();
    assert_eq!(p.server.local_max_uni, before + 1);
    assert_eq!(p.client.peer_max_uni, before + 1);
    assert!(p.server.streams.is_empty() && p.client.streams.is_empty());
  }

  #[test]
  fn the_first_frame_must_be_transport_parameters() {
    let mut server = State::new(false, &QmuxConfig::default());
    let mut record = Vec::new();
    Frame::MaxData(1).encode(&mut record);
    server.on_record(Bytes::from(record));
    assert!(matches!(&server.close, Some(CloseReason::Other(r)) if r.contains("0xa")));
  }

  #[test]
  fn connection_close_reaches_the_peer() {
    let mut p = Pair::new();
    p.client.close_local(0x10, b"bye");
    let out = p.client.build_record().unwrap();
    assert!(out.last);
    p.server.on_record(Bytes::from(out.record));
    assert_eq!(
      p.server.close,
      Some(CloseReason::ByPeer {
        code: 0x10,
        reason: "bye".to_string()
      })
    );
    assert_eq!(p.client.close, Some(CloseReason::Local));
  }
}
