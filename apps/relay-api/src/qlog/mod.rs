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

//! `/api/v1/qlog`: the relay's qlog trace files, served as written (JSON-SEQ) and
//! followed live while their connection is open, or as one qlog JSON document
//! when the request's `Accept` asks for `application/json` or
//! `application/qlog+json`.

pub mod events;
pub mod timeline;

use crate::ApiState;
use crate::http::error::ApiError;
use crate::http::list::parse_query;
use axum::Json;
use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use moqtail::transport::qlog::{self, LiveEvent};
use serde::{Deserialize, Serialize};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use tokio::sync::broadcast::error::RecvError;

const CONTENT_TYPE: &str = "application/qlog+json-seq";

/// Asks nginx, and proxies that follow its convention, to pass a streamed
/// response on as it is written. Buffered, a quiet stream never arrives.
pub(crate) const NO_PROXY_BUFFERING: (header::HeaderName, &str) =
  (header::HeaderName::from_static("x-accel-buffering"), "no");
/// The single-document form, for clients that parse one JSON value.
const CONTAINED_TYPE: &str = "application/qlog+json";
/// The JSON-SEQ record separator that starts every record.
const RS: u8 = 0x1e;

#[derive(Debug, Serialize)]
pub struct TraceFile {
  /// The connection the trace belongs to; `null` for a QUIC trace whose
  /// connection is no longer known.
  conn_id: Option<u64>,
  /// `moqt` for control messages and stream headers, `quic` for packets.
  kind: &'static str,
  file: String,
  bytes: u64,
  modified_at_ms: u64,
  /// True while the connection is open and the trace still grows.
  open: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct TraceQuery {
  /// After the file, keep the response open and send each new event.
  #[serde(default)]
  follow: bool,
  /// Only events at or after this time, in milliseconds since the Unix epoch.
  since: Option<f64>,
}

/// The index of trace files in the qlog folder, newest first.
pub async fn index(State(state): State<Arc<ApiState>>) -> Result<Json<Vec<TraceFile>>, ApiError> {
  let folder = folder(&state)?;
  let open = state.open_connections().await;
  let closed = state.reporter.closed_connections();
  let remotes: Vec<(u64, String, bool)> = open
    .iter()
    .map(|c| (c.conn_id, quic_file_suffix(&c.remote), true))
    .chain(
      closed
        .iter()
        .map(|c| (c.info.conn_id, quic_file_suffix(&c.info.remote), false)),
    )
    .collect();

  let mut files = Vec::new();
  let mut entries = tokio::fs::read_dir(folder)
    .await
    .map_err(|e| folder_error(folder, e))?;
  while let Ok(Some(entry)) = entries.next_entry().await {
    let name = entry.file_name().to_string_lossy().into_owned();
    let Ok(meta) = entry.metadata().await else {
      continue;
    };
    let (conn_id, kind) = if let Some(id) = moqt_file_conn(&name) {
      (Some(id), "moqt")
    } else if name.ends_with("_quic.sqlog") {
      let found = remotes
        .iter()
        .find(|(_, suffix, _)| name.ends_with(suffix.as_str()));
      (found.map(|(id, _, _)| *id), "quic")
    } else {
      continue;
    };
    let open = conn_id.is_some_and(|id| open.iter().any(|c| c.conn_id == id));
    files.push(TraceFile {
      conn_id,
      kind,
      file: name,
      bytes: meta.len(),
      modified_at_ms: meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64),
      open,
    });
  }
  files.sort_by_key(|f| std::cmp::Reverse(f.modified_at_ms));
  Ok(Json(files))
}

/// A connection's MoQT trace: the file's complete records, then, with `follow`,
/// each new event until the connection closes.
pub async fn trace(
  State(state): State<Arc<ApiState>>,
  Path(conn_id): Path<u64>,
  headers: HeaderMap,
  query: Result<Query<TraceQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
  let query = parse_query(query)?;
  let json = wants_json(&headers);
  if json && query.follow {
    return Err(
      ApiError::new(StatusCode::NOT_ACCEPTABLE, "follow_needs_json_seq").detail(
        "following a trace streams it as JSON-SEQ; drop follow, or accept application/qlog+json-seq",
      ),
    );
  }
  let folder = folder(&state)?;
  // Follow before reading, so no event falls between the file and the live feed.
  let live = query.follow.then(qlog::subscribe).flatten();
  let path = newest_moqt_file(folder, conn_id).await.ok_or_else(|| {
    ApiError::new(StatusCode::NOT_FOUND, "no_trace")
      .detail(format!("no qlog trace for connection {conn_id}"))
  })?;
  let content = tokio::fs::read(&path)
    .await
    .map_err(|e| folder_error(folder, e))?;
  let (records, last_time) = complete_records(&content, query.since);
  if json {
    return Ok(contained_response(&records));
  }

  let still_open = state
    .open_connections()
    .await
    .iter()
    .any(|c| c.conn_id == conn_id);
  let headers = [(header::CONTENT_TYPE, CONTENT_TYPE)];
  Ok(match live {
    Some(live) if still_open => (
      headers,
      [NO_PROXY_BUFFERING],
      Body::from_stream(follow(records, live, conn_id, last_time)),
    )
      .into_response(),
    _ => (headers, Body::from(records)).into_response(),
  })
}

/// A connection's QUIC packet trace, written by debug builds of the relay.
pub async fn quic_trace(
  State(state): State<Arc<ApiState>>,
  Path(conn_id): Path<u64>,
  headers: HeaderMap,
) -> Result<Response, ApiError> {
  let folder = folder(&state)?;
  let open = state.open_connections().await;
  let closed = state.reporter.closed_connections();
  let remote = open
    .iter()
    .chain(closed.iter().map(|c| &c.info))
    .find(|c| c.conn_id == conn_id)
    .map(|c| c.remote.clone())
    .ok_or_else(|| {
      ApiError::new(StatusCode::NOT_FOUND, "unknown_connection")
        .detail(format!("no open or recently closed connection {conn_id}"))
    })?;
  let suffix = quic_file_suffix(&remote);
  let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
  if let Ok(mut entries) = tokio::fs::read_dir(folder).await {
    while let Ok(Some(entry)) = entries.next_entry().await {
      let name = entry.file_name().to_string_lossy().into_owned();
      if name.ends_with(&suffix)
        && let Ok(modified) = entry.metadata().await.and_then(|m| m.modified())
        && newest.as_ref().is_none_or(|(t, _)| modified > *t)
      {
        newest = Some((modified, entry.path()));
      }
    }
  }
  let (_, path) = newest.ok_or_else(|| {
    ApiError::new(StatusCode::NOT_FOUND, "no_trace")
      .detail("no QUIC trace for this connection; only debug builds of the relay write them")
  })?;
  let content = tokio::fs::read(&path)
    .await
    .map_err(|e| folder_error(folder, e))?;
  let (records, _) = complete_records(&content, None);
  if wants_json(&headers) {
    return Ok(contained_response(&records));
  }
  Ok(([(header::CONTENT_TYPE, CONTENT_TYPE)], records).into_response())
}

/// Whether the request asks for one JSON document rather than JSON-SEQ. JSON-SEQ
/// stays the answer to anything else, including `*/*`.
fn wants_json(headers: &HeaderMap) -> bool {
  let accept = headers
    .get(header::ACCEPT)
    .and_then(|v| v.to_str().ok())
    .unwrap_or("")
    .to_ascii_lowercase();
  !accept.contains("json-seq")
    && (accept.contains("application/json") || accept.contains(CONTAINED_TYPE))
}

fn contained_response(records: &[u8]) -> Response {
  (
    [(header::CONTENT_TYPE, CONTAINED_TYPE)],
    Json(contained(records)),
  )
    .into_response()
}

/// One qlog document from JSON-SEQ records: the header's fields at the top level,
/// and its trace with every event in one `events` array. The first record
/// without an event name is the header. Our traces name their schema
/// (`file_schema`), quinn's QUIC traces their format (`qlog_format`); either is
/// changed to the single-document form.
fn contained(records: &[u8]) -> serde_json::Value {
  use serde_json::{Map, Value};
  let mut header: Option<Map<String, Value>> = None;
  let mut events = Vec::new();
  for record in records.split(|b| *b == RS).filter(|r| !r.is_empty()) {
    let Ok(value) = serde_json::from_slice::<Value>(record) else {
      continue;
    };
    match value {
      Value::Object(object) if header.is_none() && !object.contains_key("name") => {
        header = Some(object);
      }
      event => events.push(event),
    }
  }
  let mut header = header.unwrap_or_default();
  let mut trace = match header.remove("trace") {
    Some(Value::Object(trace)) => trace,
    _ => Map::new(),
  };
  trace.insert("events".into(), Value::Array(events));
  if header.contains_key("file_schema") {
    header.insert(
      "file_schema".into(),
      "urn:ietf:params:qlog:file:contained".into(),
    );
  }
  if header.contains_key("serialization_format") {
    header.insert("serialization_format".into(), CONTAINED_TYPE.into());
  }
  if header.get("qlog_format").and_then(Value::as_str) == Some("JSON-SEQ") {
    header.insert("qlog_format".into(), "JSON".into());
  }
  header.insert("traces".into(), Value::Array(vec![Value::Object(trace)]));
  Value::Object(header)
}

fn folder(state: &ApiState) -> Result<&FsPath, ApiError> {
  state.config.qlog_folder.as_deref().ok_or_else(|| {
    ApiError::new(StatusCode::CONFLICT, "qlog_off")
      .detail("set qlog_folder in the relay configuration to write qlog traces")
  })
}

fn folder_error(folder: &FsPath, e: std::io::Error) -> ApiError {
  ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "qlog_unreadable")
    .detail(format!("cannot read {}: {e}", folder.display()))
}

/// The connection id of a MoQT trace file: `server_conn<id>.sqlog`, or
/// `server_conn<id>_<n>.sqlog` for a later trace of a reused id.
fn moqt_file_conn(name: &str) -> Option<u64> {
  let rest = name.strip_prefix("server_conn")?.strip_suffix(".sqlog")?;
  let id = rest.split('_').next()?;
  id.parse().ok()
}

/// The end of the QUIC trace file name for a remote address, as the relay names
/// those files: the address with `:`, `[` and `]` replaced by `_`.
fn quic_file_suffix(remote: &str) -> String {
  format!("_{}_quic.sqlog", remote.replace([':', '[', ']'], "_"))
}

async fn newest_moqt_file(folder: &FsPath, conn_id: u64) -> Option<PathBuf> {
  let mut entries = tokio::fs::read_dir(folder).await.ok()?;
  let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
  while let Ok(Some(entry)) = entries.next_entry().await {
    let name = entry.file_name().to_string_lossy().into_owned();
    if moqt_file_conn(&name) == Some(conn_id)
      && let Ok(modified) = entry.metadata().await.and_then(|m| m.modified())
      && newest.as_ref().is_none_or(|(t, _)| modified > *t)
    {
      newest = Some((modified, entry.path()));
    }
  }
  newest.map(|(_, path)| path)
}

/// The complete records of a JSON-SEQ file (a record being written at this moment
/// is left out), keeping the header and the events at or after `since`. Also
/// returns the time of the last event, so a live follow can skip what was read.
fn complete_records(content: &[u8], since: Option<f64>) -> (Bytes, f64) {
  let end = content
    .iter()
    .rposition(|b| *b == b'\n')
    .map_or(0, |i| i + 1);
  let mut out = Vec::with_capacity(end);
  let mut last_time = f64::MIN;
  for record in content[..end].split(|b| *b == RS).filter(|r| !r.is_empty()) {
    let time = record_time(record);
    if let Some(time) = time {
      last_time = last_time.max(time);
    }
    let keep = match (time, since) {
      (Some(time), Some(since)) => time >= since,
      // The header has no time and is always kept.
      _ => true,
    };
    if keep {
      out.push(RS);
      out.extend_from_slice(record);
    }
  }
  (Bytes::from(out), last_time)
}

fn record_time(record: &[u8]) -> Option<f64> {
  #[derive(Deserialize)]
  struct Timed {
    time: f64,
  }
  serde_json::from_slice::<Timed>(record).ok().map(|t| t.time)
}

/// The file's records, then the connection's live events newer than the file,
/// until it closes.
fn follow(
  first: Bytes,
  live: tokio::sync::broadcast::Receiver<LiveEvent>,
  conn_id: u64,
  after: f64,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> {
  let start = futures_util::stream::once(async move { Ok(first) });
  let rest = futures_util::stream::unfold(live, move |mut live| async move {
    loop {
      match live.recv().await {
        Ok(LiveEvent::Event { conn, json }) if conn as u64 == conn_id => {
          // Already sent from the file.
          if record_time(&json).is_some_and(|t| t <= after) {
            continue;
          }
          let mut record = Vec::with_capacity(json.len() + 2);
          record.push(RS);
          record.extend_from_slice(&json);
          record.push(b'\n');
          return Some((Ok(Bytes::from(record)), live));
        }
        Ok(LiveEvent::Closed(conn)) if conn as u64 == conn_id => return None,
        Ok(_) | Err(RecvError::Lagged(_)) => continue,
        Err(RecvError::Closed) => return None,
      }
    }
  });
  futures_util::StreamExt::chain(start, rest)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn json_seq_records_become_one_contained_document() {
    let seq = [
      r#"{"file_schema":"urn:ietf:params:qlog:file:sequential","serialization_format":"application/qlog+json-seq","title":"t","trace":{"vantage_point":{"type":"server"}}}"#,
      r#"{"time":1.0,"name":"moqt:control_message_parsed","data":{}}"#,
      r#"{"time":2.0,"name":"moqtail:connection_closed","data":{}}"#,
    ]
    .iter()
    .flat_map(|r| [&[RS][..], r.as_bytes(), b"\n"].concat())
    .collect::<Vec<u8>>();
    let doc = contained(&seq);
    assert_eq!(doc["file_schema"], "urn:ietf:params:qlog:file:contained");
    assert_eq!(doc["serialization_format"], "application/qlog+json");
    assert_eq!(doc["title"], "t");
    assert!(doc.get("trace").is_none());
    let trace = &doc["traces"][0];
    assert_eq!(trace["vantage_point"]["type"], "server");
    assert_eq!(trace["events"].as_array().unwrap().len(), 2);
    assert_eq!(trace["events"][1]["name"], "moqtail:connection_closed");
  }

  #[test]
  fn json_is_served_only_when_asked_for_without_json_seq() {
    let accept = |v: &str| {
      let mut h = HeaderMap::new();
      h.insert(header::ACCEPT, v.parse().unwrap());
      wants_json(&h)
    };
    assert!(accept("application/json"));
    assert!(accept("application/qlog+json"));
    assert!(!accept("*/*"));
    assert!(!accept("application/qlog+json-seq"));
    assert!(!accept("application/qlog+json-seq, application/json"));
    assert!(!wants_json(&HeaderMap::new()));
  }

  #[test]
  fn file_names() {
    assert_eq!(moqt_file_conn("server_conn42.sqlog"), Some(42));
    assert_eq!(moqt_file_conn("server_conn42_3.sqlog"), Some(42));
    assert_eq!(moqt_file_conn("client_123_conn0.sqlog"), None);
    assert_eq!(moqt_file_conn("server_conn42.txt"), None);
    assert_eq!(
      quic_file_suffix("127.0.0.1:5000"),
      "_127.0.0.1_5000_quic.sqlog"
    );
    assert_eq!(quic_file_suffix("[::1]:5000"), "____1__5000_quic.sqlog");
  }

  #[test]
  fn complete_records_drop_a_partial_last_record_and_filter_by_time() {
    let mut file = Vec::new();
    for record in [
      r#"{"file_schema":"x"}"#,
      r#"{"time":10.0}"#,
      r#"{"time":20.0}"#,
    ] {
      file.push(RS);
      file.extend_from_slice(record.as_bytes());
      file.push(b'\n');
    }
    file.extend_from_slice(b"\x1e{\"time\":30");

    let (all, last) = complete_records(&file, None);
    assert_eq!(all.iter().filter(|b| **b == RS).count(), 3);
    assert_eq!(last, 20.0);

    let (recent, _) = complete_records(&file, Some(15.0));
    let text = String::from_utf8(recent.to_vec()).unwrap();
    assert!(text.contains("file_schema") && text.contains("20.0") && !text.contains("10.0"));
  }
}
