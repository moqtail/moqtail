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

//! qlog traces of MoQT control messages, one JSON-SEQ (`.sqlog`) file per connection.
//!
//! Off until [`init`] or [`init_live`] is called. Callers only build the event JSON;
//! a single writer thread does all file I/O, and events are dropped rather than ever
//! blocking the caller when it falls behind. The same events can also be followed
//! live with [`subscribe`].

use crate::model::common::location::Location;
use crate::model::common::pair::KeyValuePair;
use crate::model::common::tuple::{Tuple, TupleField};
use crate::model::control::control_message::ControlMessage;
use crate::model::data::fetch_header::FetchHeader;
use crate::model::data::subgroup_header::SubgroupHeader;
use crate::model::parameter::authorization_token::AuthorizationToken;
use crate::model::parameter::location_filter::LocationFilter;
use crate::model::parameter::message_parameter::MessageParameter;
use crate::model::property::track_property::TrackProperty;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tracing::warn;

const MOQT_EVENT_SCHEMA: &str = "urn:ietf:params:qlog:events:moqt-08";
const LOGLEVEL_EVENT_SCHEMA: &str = "urn:ietf:params:qlog:events:loglevel";
/// Events buffered for the writer thread before new ones are dropped.
const QUEUE_CAPACITY: usize = 65_536;
/// Events buffered for each live follower; a slower follower misses the oldest.
const LIVE_CAPACITY: usize = 4096;

struct Qlog {
  /// The writer thread's queue; `None` when no files are written.
  files: Option<SyncSender<Record>>,
  live: broadcast::Sender<LiveEvent>,
  dropped: AtomicU64,
}

/// What a live follower receives.
#[derive(Debug, Clone)]
pub enum LiveEvent {
  /// One qlog event of a connection, as the JSON written to its file.
  Event { conn: usize, json: Arc<[u8]> },
  /// The connection closed; its trace has no more events.
  Closed(usize),
}

enum Record {
  Event { conn: usize, json: Vec<u8> },
  Close(usize),
  Flush(SyncSender<()>),
}

static QLOG: OnceLock<Qlog> = OnceLock::new();

/// Starts writing qlog files into `dir`. `vantage` is the qlog vantage point type,
/// `"server"` or `"client"`. Later calls are ignored.
pub fn init(dir: impl Into<PathBuf>, vantage: &'static str) -> std::io::Result<()> {
  let dir = dir.into();
  std::fs::create_dir_all(&dir)?;
  let (tx, rx) = sync_channel(QUEUE_CAPACITY);
  if QLOG
    .set(Qlog {
      files: Some(tx),
      live: broadcast::channel(LIVE_CAPACITY).0,
      dropped: AtomicU64::new(0),
    })
    .is_ok()
  {
    std::thread::Builder::new()
      .name("qlog-writer".into())
      .spawn(move || write_loop(rx, &dir, vantage))?;
  }
  Ok(())
}

/// Produces events for live followers only, writing no files. Does nothing when
/// qlog is already on.
pub fn init_live() {
  let _ = QLOG.set(Qlog {
    files: None,
    live: broadcast::channel(LIVE_CAPACITY).0,
    dropped: AtomicU64::new(0),
  });
}

/// Follows every event as it is produced. `None` until qlog is on. Without files,
/// events are built only while at least one follower exists.
pub fn subscribe() -> Option<broadcast::Receiver<LiveEvent>> {
  QLOG.get().map(|q| q.live.subscribe())
}

/// True while events are being produced: files are written or someone follows.
pub fn enabled() -> bool {
  active().is_some()
}

fn active() -> Option<&'static Qlog> {
  QLOG
    .get()
    .filter(|q| q.files.is_some() || q.live.receiver_count() > 0)
}

/// Flushes and closes the connection's file. Its next event opens a new one.
pub fn close(conn: usize) {
  if let Some(q) = QLOG.get() {
    q.send(Record::Close(conn));
    let _ = q.live.send(LiveEvent::Closed(conn));
  }
}

/// Writes out every queued event, waiting up to `timeout`. Call before the process
/// exits, since the writer thread does not outlive it.
pub fn flush(timeout: std::time::Duration) {
  if let Some(files) = QLOG.get().and_then(|q| q.files.as_ref()) {
    let (tx, rx) = sync_channel(1);
    if files.send(Record::Flush(tx)).is_ok() {
      let _ = rx.recv_timeout(timeout);
    }
  }
}

impl Qlog {
  /// Hands one event to the file writer and to the live followers.
  fn emit(&self, conn: usize, json: Vec<u8>) {
    if self.live.receiver_count() > 0 {
      let _ = self.live.send(LiveEvent::Event {
        conn,
        json: Arc::from(json.as_slice()),
      });
    }
    self.send(Record::Event { conn, json });
  }

  fn send(&self, record: Record) {
    let Some(files) = &self.files else { return };
    if let Err(TrySendError::Full(_)) = files.try_send(record) {
      let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
      if n.is_power_of_two() {
        warn!("qlog writer is behind, {n} events dropped so far");
      }
    }
  }
}

/// Records a control message sent (`created`) or received on a stream.
/// [`ControlStreamHandler`](super::control_stream_handler::ControlStreamHandler) calls
/// this itself; call it only for a message written to the stream directly.
pub fn control_message(
  conn: usize,
  stream_id: u64,
  created: bool,
  message: &ControlMessage,
  raw: &[u8],
) {
  let Some(q) = active() else { return };
  let name = if created {
    "moqt:control_message_created"
  } else {
    "moqt:control_message_parsed"
  };
  let data = json!({
    "stream_id": stream_id,
    "length": raw.len(),
    "message": message_json(message),
    "raw": raw_info(raw),
  });
  q.emit(conn, event(name, data));
}

/// Records the header a data stream begins with, sent (`created`) or received, from
/// its wire bytes: a subgroup or a fetch header. One event per stream, not per object.
pub fn data_stream_header(conn: usize, stream_id: u64, created: bool, raw: &[u8]) {
  let Some(q) = active() else { return };
  let Some((name, data)) = stream_header_json(stream_id, raw) else {
    return;
  };
  let suffix = if created { "created" } else { "parsed" };
  q.emit(conn, event(&format!("moqt:{name}_{suffix}"), data));
}

fn stream_header_json(stream_id: u64, raw: &[u8]) -> Option<(&'static str, Value)> {
  let mut bytes = bytes::Bytes::copy_from_slice(raw);
  let (name, data) = if let Ok(h) = SubgroupHeader::deserialize(&mut bytes.clone()) {
    let ty = h.header_type;
    let mut data = json!({
      "stream_id": stream_id,
      "track_alias": h.track_alias,
      "group_id": h.group_id,
      "subgroup_id_mode": (ty.value() & 0x06) >> 1,
      "contains_end_of_group": ty.contains_end_of_group(),
      "properties_present": ty.has_properties(),
    });
    if let Some(id) = h.subgroup_id {
      data["subgroup_id"] = json!(id);
    }
    if let Some(p) = h.publisher_priority {
      data["publisher_priority"] = json!(p);
    }
    ("subgroup_header", data)
  } else if let Ok(h) = FetchHeader::deserialize(&mut bytes) {
    (
      "fetch_header",
      json!({ "stream_id": stream_id, "request_id": h.request_id }),
    )
  } else {
    return None;
  };
  Some((name, data))
}

/// Records bytes on a control or request stream that did not parse.
pub(crate) fn parse_error(conn: usize, stream_id: u64, raw: &[u8]) {
  let Some(q) = active() else { return };
  let data = json!({
    "message": format!("control message on stream {stream_id} failed to parse"),
    "raw": raw_info(raw),
  });
  q.emit(conn, event("loglevel:error", data));
}

fn event(name: &str, data: Value) -> Vec<u8> {
  let time = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs_f64() * 1000.0)
    .unwrap_or(0.0);
  serde_json::to_vec(&json!({ "time": time, "name": name, "data": data })).unwrap_or_default()
}

fn write_loop(rx: Receiver<Record>, dir: &Path, vantage: &str) {
  let mut files: HashMap<usize, BufWriter<File>> = HashMap::new();
  // Block for the first record, drain whatever else is queued, then flush once.
  while let Ok(first) = rx.recv() {
    let mut next = Some(first);
    while let Some(record) = next {
      match record {
        Record::Event { conn, json } => {
          let file = match files.entry(conn) {
            std::collections::hash_map::Entry::Occupied(e) => Some(e.into_mut()),
            std::collections::hash_map::Entry::Vacant(e) => match open(dir, vantage, conn) {
              Ok(w) => Some(e.insert(w)),
              Err(err) => {
                warn!("qlog: cannot open file for conn {conn}: {err}");
                None
              }
            },
          };
          if let Some(w) = file {
            let _ = w.write_all(b"\x1e");
            let _ = w.write_all(&json);
            let _ = w.write_all(b"\n");
          }
        }
        Record::Close(conn) => {
          if let Some(mut w) = files.remove(&conn) {
            let _ = w.flush();
          }
        }
        Record::Flush(done) => {
          for w in files.values_mut() {
            let _ = w.flush();
          }
          let _ = done.send(());
        }
      }
      next = rx.try_recv().ok();
    }
    for w in files.values_mut() {
      let _ = w.flush();
    }
  }
}

fn open(dir: &Path, vantage: &str, conn: usize) -> std::io::Result<BufWriter<File>> {
  // A relay's connection ids are unique per run, so its files are named by id alone.
  // A client is always connection 0, so its files carry the start time.
  let base = if vantage == "server" {
    format!("{vantage}_conn{conn}")
  } else {
    let started = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_millis())
      .unwrap_or(0);
    format!("{vantage}_{started}_conn{conn}")
  };
  // Never overwrite an earlier trace: an id repeated across restarts gets a suffix.
  let mut group_id = base.clone();
  let mut n = 1;
  let file = loop {
    match std::fs::OpenOptions::new()
      .write(true)
      .create_new(true)
      .open(dir.join(format!("{group_id}.sqlog")))
    {
      Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
        n += 1;
        group_id = format!("{base}_{n}");
      }
      other => break other?,
    }
  };
  let mut w = BufWriter::new(file);
  let header = json!({
    "file_schema": "urn:ietf:params:qlog:file:sequential",
    "serialization_format": "application/qlog+json-seq",
    "title": format!("moqtail {vantage} connection {conn}"),
    "trace": {
      "vantage_point": { "name": "moqtail", "type": vantage },
      "event_schemas": [MOQT_EVENT_SCHEMA, LOGLEVEL_EVENT_SCHEMA],
      "common_fields": {
        "group_id": group_id,
        "time_format": "relative_to_epoch",
        "reference_time": { "clock_type": "system", "epoch": "1970-01-01T00:00:00.000Z" },
      },
    },
  });
  w.write_all(b"\x1e")?;
  serde_json::to_writer(&mut w, &header)?;
  w.write_all(b"\n")?;
  Ok(w)
}

fn hex(bytes: &[u8]) -> String {
  use std::fmt::Write;
  bytes
    .iter()
    .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
      let _ = write!(s, "{b:02x}");
      s
    })
}

fn raw_info(bytes: &[u8]) -> Value {
  json!({ "length": bytes.len(), "data": hex(bytes) })
}

fn byte_string(field: &TupleField) -> Value {
  match std::str::from_utf8(field.as_bytes()) {
    Ok(text) => json!({ "value": text }),
    Err(_) => json!({ "value_bytes": hex(field.as_bytes()) }),
  }
}

fn tuple(t: &Tuple) -> Value {
  Value::Array(t.fields.iter().map(byte_string).collect())
}

fn location(l: &Location) -> Value {
  json!({ "group": l.group, "object": l.object })
}

fn kvp_value(entry: &mut Map<String, Value>, kvp: &KeyValuePair) {
  match kvp {
    KeyValuePair::VarInt { value, .. } => {
      entry.insert("value".into(), json!(value));
    }
    KeyValuePair::Bytes { value, .. } => {
      entry.insert("length".into(), json!(value.len()));
      entry.insert("value_bytes".into(), raw_info(value));
    }
  }
}

fn auth_token(entry: &mut Map<String, Value>, token: &AuthorizationToken) {
  let alias_type = match token.alias_type {
    0 => "delete",
    1 => "register",
    2 => "use_alias",
    _ => "use_value",
  };
  entry.insert("alias_type".into(), json!(alias_type));
  if let Some(alias) = token.token_alias {
    entry.insert("token_alias".into(), json!(alias));
  }
  if let Some(ty) = token.token_type {
    entry.insert("token_type".into(), json!(ty));
  }
  if let Some(value) = &token.token_value {
    entry.insert("token_value".into(), raw_info(value));
  }
}

fn location_filter(filter: &LocationFilter) -> Value {
  match filter {
    LocationFilter::None => json!({ "filter_type": 0 }),
    LocationFilter::RelativeStart { start_group } => {
      json!({ "filter_type": 1, "start_group": start_group })
    }
    LocationFilter::AbsoluteStart { start } => {
      json!({ "filter_type": 2, "start_location": location(start) })
    }
    LocationFilter::AbsoluteStartGroupEnd { start, end_group } => json!({
      "filter_type": 3,
      "start_location": location(start),
      "end_group_delta": end_group.saturating_sub(start.group),
    }),
    LocationFilter::AbsoluteRange { start, end } => json!({
      "filter_type": 4,
      "start_location": location(start),
      "end_location": location(end),
    }),
    LocationFilter::NextObject => json!({ "filter_type": 5 }),
  }
}

fn parameter(p: &MessageParameter) -> Value {
  let mut e = Map::new();
  let named = |e: &mut Map<String, Value>, name: &str, value: Value| {
    e.insert("name".into(), json!(name));
    e.insert("value".into(), value);
  };
  match p {
    MessageParameter::ObjectDeliveryTimeout { timeout } => {
      named(&mut e, "object_delivery_timeout", json!(timeout))
    }
    MessageParameter::SubgroupDeliveryTimeout { timeout } => {
      named(&mut e, "subgroup_delivery_timeout", json!(timeout))
    }
    MessageParameter::RendezvousTimeout { timeout } => {
      named(&mut e, "rendezvous_timeout", json!(timeout))
    }
    MessageParameter::FillTimeout { timeout } => named(&mut e, "fill_timeout", json!(timeout)),
    MessageParameter::Expires { expires } => named(&mut e, "expires", json!(expires)),
    MessageParameter::LargestObject { location: l } => named(&mut e, "largest_object", location(l)),
    MessageParameter::Forward { forward } => named(&mut e, "forward", json!(*forward as u64)),
    MessageParameter::SubscriberPriority { priority } => {
      named(&mut e, "subscriber_priority", json!(priority))
    }
    MessageParameter::GroupOrder { order } => named(&mut e, "group_order", json!(u8::from(*order))),
    MessageParameter::LocationFilter { filter } => {
      named(&mut e, "subscription_filter", location_filter(filter))
    }
    MessageParameter::NewGroupRequest { group } => named(&mut e, "new_group_request", json!(group)),
    MessageParameter::TrackNamespacePrefix { prefix } => {
      named(&mut e, "track_namespace_prefix", tuple(prefix))
    }
    MessageParameter::AuthorizationToken { token } => {
      e.insert("name".into(), json!("authorization_token"));
      auth_token(&mut e, token);
    }
    // Parameters the qlog draft has no name for: the wire form, as for unknown ones.
    other => {
      e.insert("name".into(), json!("unknown"));
      e.insert("type".into(), json!(other.type_value()));
      if let Ok(kvp) = TryInto::<KeyValuePair>::try_into(other.clone()) {
        kvp_value(&mut e, &kvp);
      }
    }
  }
  Value::Object(e)
}

fn setup_option(kvp: &KeyValuePair) -> Value {
  let mut e = Map::new();
  let text = |b: &[u8]| json!(String::from_utf8_lossy(b));
  match (kvp.get_type(), kvp) {
    (0x01, KeyValuePair::Bytes { value, .. }) => {
      e.insert("name".into(), json!("path"));
      e.insert("value".into(), text(value));
    }
    (0x05, KeyValuePair::Bytes { value, .. }) => {
      e.insert("name".into(), json!("authority"));
      e.insert("value".into(), text(value));
    }
    (0x07, KeyValuePair::Bytes { value, .. }) => {
      e.insert("name".into(), json!("implementation"));
      e.insert("value".into(), text(value));
    }
    (0x04, KeyValuePair::VarInt { value, .. }) => {
      e.insert("name".into(), json!("max_auth_token_cache_size"));
      e.insert("value".into(), json!(value));
    }
    (ty, kvp) => {
      e.insert("name".into(), json!("unknown"));
      e.insert("type".into(), json!(ty));
      kvp_value(&mut e, kvp);
    }
  }
  Value::Object(e)
}

fn property(p: &TrackProperty) -> Value {
  let mut e = Map::new();
  e.insert("type".into(), json!(p.type_value()));
  if let Ok(kvp) = TryInto::<KeyValuePair>::try_into(p.clone()) {
    kvp_value(&mut e, &kvp);
  }
  Value::Object(e)
}

fn params(e: &mut Map<String, Value>, ps: &[MessageParameter]) {
  e.insert("number_of_parameters".into(), json!(ps.len()));
  if !ps.is_empty() {
    e.insert(
      "parameters".into(),
      Value::Array(ps.iter().map(parameter).collect()),
    );
  }
}

fn props(e: &mut Map<String, Value>, ps: &[TrackProperty]) {
  if !ps.is_empty() {
    e.insert(
      "track_properties".into(),
      Value::Array(ps.iter().map(property).collect()),
    );
  }
}

/// The qlog `$MOQTControlMessage` form of a message.
fn message_json(message: &ControlMessage) -> Value {
  let mut e = Map::new();
  let mut set = |k: &str, v: Value| {
    e.insert(k.into(), v);
  };
  let ty = match message {
    ControlMessage::Setup(m) => {
      set(
        "setup_options",
        Value::Array(m.setup_options.iter().map(setup_option).collect()),
      );
      "setup"
    }
    ControlMessage::Goaway(m) => {
      set(
        "new_session_uri",
        raw_info(m.new_session_uri.as_deref().unwrap_or("").as_bytes()),
      );
      set("timeout", json!(m.timeout));
      "goaway"
    }
    ControlMessage::RequestOk(m) => {
      params(&mut e, &m.parameters);
      props(&mut e, &m.track_properties);
      "request_ok"
    }
    ControlMessage::RequestError(m) => {
      set("error_code", json!(u64::from(m.error_code)));
      set("retry_interval", json!(m.retry_interval));
      set("reason_phrase", json!(m.reason_phrase.as_str()));
      if let Some(r) = &m.redirect {
        set(
          "redirect",
          json!({
            "connect_uri": raw_info(r.connect_uri.as_deref().unwrap_or("").as_bytes()),
            "track_namespace": tuple(&r.track_namespace),
            "track_name": byte_string(&r.track_name),
          }),
        );
      }
      "request_error"
    }
    ControlMessage::Subscribe(m) => {
      set("request_id", json!(m.request_id));
      set("track_namespace", tuple(&m.track_namespace));
      set("track_name", byte_string(&m.track_name));
      params(&mut e, &m.subscribe_parameters);
      "subscribe"
    }
    ControlMessage::SubscribeOk(m) => {
      set("track_alias", json!(m.track_alias));
      params(&mut e, &m.subscribe_parameters);
      props(&mut e, &m.track_properties);
      "subscribe_ok"
    }
    ControlMessage::RequestUpdate(m) => {
      set("request_id", json!(m.request_id));
      params(&mut e, &m.parameters);
      "request_update"
    }
    ControlMessage::Publish(m) => {
      set("request_id", json!(m.request_id));
      set("track_namespace", tuple(&m.track_namespace));
      set("track_name", byte_string(&m.track_name));
      set("track_alias", json!(m.track_alias));
      params(&mut e, &m.parameters);
      props(&mut e, &m.track_properties);
      "publish"
    }
    ControlMessage::PublishDone(m) => {
      set("status_code", json!(u64::from(m.status_code)));
      set("stream_count", json!(m.stream_count));
      set("reason_phrase", json!(m.reason_phrase.as_str()));
      "publish_done"
    }
    ControlMessage::Fetch(m) => {
      set("request_id", json!(m.request_id));
      // The range travels in parameters in this protocol version.
      set("fetch_type", json!("standalone"));
      set(
        "standalone_fetch",
        json!({
          "track_namespace": tuple(&m.track_namespace),
          "track_name": byte_string(&m.track_name),
        }),
      );
      params(&mut e, &m.parameters);
      "fetch"
    }
    ControlMessage::FetchOk(m) => {
      set("end_of_track", json!(m.end_of_track as u8));
      set("end_location", location(&m.end_location));
      params(&mut e, &m.subscribe_parameters);
      props(&mut e, &m.track_properties);
      "fetch_ok"
    }
    ControlMessage::TrackStatus(m) => {
      set("request_id", json!(m.request_id));
      set("track_namespace", tuple(&m.track_namespace));
      set("track_name", byte_string(&m.track_name));
      params(&mut e, &m.subscribe_parameters);
      "track_status"
    }
    ControlMessage::PublishNamespace(m) => {
      set("request_id", json!(m.request_id));
      set("track_namespace", tuple(&m.track_namespace));
      params(&mut e, &m.parameters);
      "publish_namespace"
    }
    ControlMessage::Namespace(m) => {
      set("track_namespace_suffix", tuple(&m.track_namespace_suffix));
      if let Some(ps) = &m.parameters {
        params(&mut e, ps);
      }
      "namespace"
    }
    ControlMessage::NamespaceDone(m) => {
      set("track_namespace_suffix", tuple(&m.track_namespace_suffix));
      "namespace_done"
    }
    ControlMessage::SubscribeNamespace(m) => {
      set("request_id", json!(m.request_id));
      set("track_namespace_prefix", tuple(&m.track_namespace_prefix));
      params(&mut e, &m.parameters);
      "subscribe_namespace"
    }
    ControlMessage::SubscribeTracks(m) => {
      set("request_id", json!(m.request_id));
      set("track_namespace_prefix", tuple(&m.track_namespace_prefix));
      params(&mut e, &m.parameters);
      "subscribe_tracks"
    }
    ControlMessage::PublishSkipped(m) => {
      set("track_namespace_suffix", tuple(&m.track_namespace_suffix));
      set("track_name", byte_string(&m.track_name));
      "publish_blocked"
    }
  };
  e.insert("type".into(), json!(ty));
  Value::Object(e)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::control::subscribe::Subscribe;

  #[test]
  fn subscribe_maps_to_qlog_fields() {
    let msg = ControlMessage::Subscribe(Box::new(Subscribe {
      request_id: 4,
      track_namespace: Tuple::from_utf8_path("live/room"),
      track_name: TupleField::new(bytes::Bytes::from_static(&[0xff, 0x00])),
      subscribe_parameters: vec![
        MessageParameter::Forward { forward: true },
        MessageParameter::LocationFilter {
          filter: LocationFilter::NextObject,
        },
      ],
    }));
    assert_eq!(
      message_json(&msg),
      json!({
        "type": "subscribe",
        "request_id": 4,
        "track_namespace": [{ "value": "live" }, { "value": "room" }],
        "track_name": { "value_bytes": "ff00" },
        "number_of_parameters": 2,
        "parameters": [
          { "name": "forward", "value": 1 },
          { "name": "subscription_filter", "value": { "filter_type": 5 } },
        ],
      })
    );
  }

  #[test]
  fn stream_headers_map_to_qlog_fields() {
    let subgroup = SubgroupHeader::new_with_explicit_id(9, 3, 2, Some(7), true, false, false);
    let raw = subgroup.serialize(None).unwrap();
    let (name, data) = stream_header_json(5, &raw).unwrap();
    assert_eq!(name, "subgroup_header");
    assert_eq!(data["stream_id"], 5);
    assert_eq!(data["track_alias"], 9);
    assert_eq!(data["group_id"], 3);
    assert_eq!(data["subgroup_id"], 2);
    assert_eq!(data["publisher_priority"], 7);

    let raw = FetchHeader::new(11).serialize().unwrap();
    let (name, data) = stream_header_json(8, &raw).unwrap();
    assert_eq!(name, "fetch_header");
    assert_eq!(data, json!({ "stream_id": 8, "request_id": 11 }));

    assert!(stream_header_json(1, &[0xff]).is_none());
  }

  /// qlog is process-wide and starts once, so every test starts it the same way.
  fn start_for_tests() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("moqtail-qlog-test-{}", std::process::id()));
    init(&dir, "server").unwrap();
    dir
  }

  #[test]
  fn live_followers_get_each_event_and_the_close() {
    start_for_tests();
    let mut follower = subscribe().unwrap();
    let msg = ControlMessage::NamespaceDone(Box::new(
      crate::model::control::namespace_done::NamespaceDone {
        track_namespace_suffix: Tuple::from_utf8_path("b"),
      },
    ));
    control_message(41, 3, false, &msg, &[0x09]);
    close(41);
    let mut seen_event = false;
    loop {
      match follower.try_recv() {
        Ok(LiveEvent::Event { conn: 41, json }) => {
          let event: Value = serde_json::from_slice(&json).unwrap();
          assert_eq!(event["name"], "moqt:control_message_parsed");
          seen_event = true;
        }
        Ok(LiveEvent::Closed(41)) => break,
        Ok(_) => continue,
        Err(e) => panic!("follower stopped before the close: {e:?}"),
      }
    }
    assert!(seen_event);
  }

  #[test]
  fn writes_json_seq_file_per_connection() {
    let dir = start_for_tests();
    let msg = ControlMessage::NamespaceDone(Box::new(
      crate::model::control::namespace_done::NamespaceDone {
        track_namespace_suffix: Tuple::from_utf8_path("a"),
      },
    ));
    control_message(7, 3, true, &msg, &[0x01, 0x02]);
    close(7);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let text = loop {
      let file = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().ends_with("_conn7.sqlog"));
      if let Some(f) = file {
        let text = std::fs::read_to_string(f.path()).unwrap();
        if text.matches('\x1e').count() == 2 {
          break text;
        }
      }
      assert!(
        std::time::Instant::now() < deadline,
        "qlog file not written"
      );
      std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let records: Vec<Value> = text
      .split('\x1e')
      .skip(1)
      .map(|r| serde_json::from_str(r.trim_end()).unwrap())
      .collect();
    assert_eq!(
      records[0]["file_schema"],
      "urn:ietf:params:qlog:file:sequential"
    );
    assert_eq!(records[1]["name"], "moqt:control_message_created");
    assert_eq!(records[1]["data"]["stream_id"], 3);
    assert_eq!(records[1]["data"]["message"]["type"], "namespace_done");
    assert_eq!(records[1]["data"]["raw"]["data"], "0102");

    // The same id again (as after a restart) gets its own file.
    control_message(7, 3, true, &msg, &[0x01, 0x02]);
    close(7);
    let second = dir.join("server_conn7_2.sqlog");
    while !second.exists() {
      assert!(
        std::time::Instant::now() < deadline,
        "second qlog file not written"
      );
      std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(dir.join("server_conn7.sqlog").exists());
    let _ = std::fs::remove_dir_all(&dir);
  }
}
