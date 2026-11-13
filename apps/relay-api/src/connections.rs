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

//! `/api/v1/connections`: who is connected, and who was.

use crate::ApiState;
use crate::control::{MessageCounts, RequestCounts};
use crate::error::ApiError;
use crate::list::{ListQuery, Page, SortValue, page, parse_query};
use crate::probe::Connection;
use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use moqtail_probe::{CloseInfo, ConnectionInfo, ConnectionState, QuicStats, Role};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const SORT_KEYS: &[&str] = &[
  "conn_id",
  "connected_at",
  "rtt",
  "bytes_out",
  "bytes_in",
  "lost_packets",
  "subscriptions",
];

/// The list parameters plus `closed`. Spelled out rather than flattened: a
/// flattened struct makes the query parser treat every number as text.
#[derive(Debug, Default, Deserialize)]
pub struct ConnectionsQuery {
  /// Lists the closed connections instead of the open ones.
  #[serde(default)]
  closed: bool,
  sort: Option<String>,
  limit: Option<usize>,
  offset: Option<usize>,
  prefix: Option<String>,
}

/// One row of the connection list: the connection without its long lists.
#[derive(Debug, Serialize)]
pub struct ConnectionSummary {
  conn_id: u64,
  state: ConnectionState,
  transport: String,
  remote: String,
  role: Role,
  connected_at_ms: u64,
  implementation: Option<String>,
  announced_namespaces: usize,
  published_tracks: usize,
  subscriptions: usize,
  quic: QuicStats,
  close: Option<CloseInfo>,
  control_messages: MessageCounts,
  requests: RequestCounts,
}

impl From<&Connection> for ConnectionSummary {
  fn from(connection: &Connection) -> Self {
    let c = &connection.info;
    ConnectionSummary {
      conn_id: c.conn_id,
      state: c.state,
      transport: c.transport.clone(),
      remote: c.remote.clone(),
      role: c.role,
      connected_at_ms: c.connected_at_ms,
      implementation: c.implementation.clone(),
      announced_namespaces: c.announced_namespaces.len(),
      published_tracks: c.published_tracks.len(),
      subscriptions: c.subscriptions.len(),
      quic: c.quic.clone(),
      close: c.close.clone(),
      control_messages: connection.counts.control_messages.clone(),
      requests: connection.counts.requests.clone(),
    }
  }
}

pub async fn list(
  State(state): State<Arc<ApiState>>,
  query: Result<Query<ConnectionsQuery>, QueryRejection>,
) -> Result<Json<Page<ConnectionSummary>>, ApiError> {
  let query = parse_query(query)?;
  let list = ListQuery {
    sort: query.sort,
    limit: query.limit,
    offset: query.offset,
    prefix: query.prefix,
  };
  let connections: Vec<Connection> = if query.closed {
    state.probe.closed_connections()
  } else {
    state.open_connections_with_counts().await
  };
  let matching: Vec<&Connection> = match &list.prefix {
    Some(prefix) => connections
      .iter()
      .filter(|c| concerns_namespace(&c.info, prefix))
      .collect(),
    None => connections.iter().collect(),
  };
  let page = page(matching, &list, SORT_KEYS, |c, key| {
    let c = &c.info;
    Some(match key {
      "conn_id" => SortValue::Number(c.conn_id as f64),
      "connected_at" => SortValue::Number(c.connected_at_ms as f64),
      "rtt" => SortValue::Number(c.quic.rtt_ms),
      "bytes_out" => SortValue::Number(c.quic.sent_bytes as f64),
      "bytes_in" => SortValue::Number(c.quic.received_bytes as f64),
      "lost_packets" => SortValue::Number(c.quic.lost_packets as f64),
      "subscriptions" => SortValue::Number(c.subscriptions.len() as f64),
      _ => return None,
    })
  })?;
  Ok(Json(Page {
    total: page.total,
    offset: page.offset,
    limit: page.limit,
    items: page
      .items
      .into_iter()
      .map(ConnectionSummary::from)
      .collect(),
  }))
}

/// One connection with all its details, open or recently closed.
pub async fn detail(
  State(state): State<Arc<ApiState>>,
  Path(conn_id): Path<u64>,
) -> Result<Json<serde_json::Value>, ApiError> {
  let open = state.open_connections_with_counts().await;
  if let Some(found) = open.into_iter().find(|c| c.info.conn_id == conn_id) {
    let mut body = serde_json::json!(found);
    if let Some(subscriptions) = body["subscriptions"].as_array_mut() {
      for s in subscriptions {
        state.add_subscription_processing(conn_id, s);
      }
    }
    return Ok(Json(body));
  }
  state
    .probe
    .closed_connections()
    .into_iter()
    .find(|c| c.info.conn_id == conn_id)
    .map(|c| Json(serde_json::json!(c)))
    .ok_or_else(|| {
      ApiError::new(StatusCode::NOT_FOUND, "unknown_connection")
        .detail(format!("no open or recently closed connection {conn_id}"))
    })
}

/// True when the connection announced, publishes or subscribes to something in a
/// namespace starting with `prefix`.
fn concerns_namespace(c: &ConnectionInfo, prefix: &str) -> bool {
  c.announced_namespaces
    .iter()
    .any(|ns| ns.starts_with(prefix))
    || c
      .published_tracks
      .iter()
      .any(|t| t.track.starts_with(prefix))
    || c.subscriptions.iter().any(|s| s.track.starts_with(prefix))
}
