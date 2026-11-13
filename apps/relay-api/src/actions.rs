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

//! The `actions` feature: changes to the running relay. Every handler here needs
//! the write token (see `http::access`), and every change is logged with the address
//! that asked for it.

use crate::ApiState;
use crate::http::error::ApiError;
use crate::http::list::parse_query;
use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

/// How long a client told to go away is given when the request does not say.
const DEFAULT_TIMEOUT_MS: u64 = 10_000;
/// The longest a client may be given; longer would keep a drained relay up for
/// longer than anyone waits.
const MAX_TIMEOUT_MS: u64 = 600_000;
/// Longer URIs do not fit in a GOAWAY message.
const MAX_URI_BYTES: usize = 8192;

#[derive(Debug, Default, Deserialize)]
pub struct TimeoutQuery {
  timeout_ms: Option<u64>,
}

fn timeout(query: Result<Query<TimeoutQuery>, QueryRejection>) -> Result<u64, ApiError> {
  let timeout_ms = parse_query(query)?.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
  if timeout_ms > MAX_TIMEOUT_MS {
    return Err(
      ApiError::new(StatusCode::BAD_REQUEST, "bad_query")
        .detail(format!("timeout_ms is at most {MAX_TIMEOUT_MS}")),
    );
  }
  Ok(timeout_ms)
}

fn parse_body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
  body
    .map(|Json(b)| b)
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "bad_body").detail(e.body_text()))
}

fn no_connection(conn_id: u64) -> ApiError {
  ApiError::new(StatusCode::NOT_FOUND, "not_found")
    .detail(format!("no open connection has id {conn_id}"))
}

/// `POST /connections/{id}/goaway?timeout_ms=`: asks one client to move.
pub async fn goaway(
  State(state): State<Arc<ApiState>>,
  ConnectInfo(from): ConnectInfo<SocketAddr>,
  Path(conn_id): Path<u64>,
  query: Result<Query<TimeoutQuery>, QueryRejection>,
) -> Result<StatusCode, ApiError> {
  let timeout_ms = timeout(query)?;
  if !state.actions.goaway(conn_id, timeout_ms).await {
    return Err(no_connection(conn_id));
  }
  info!("API action from {from}: GOAWAY to connection {conn_id}, timeout {timeout_ms} ms");
  Ok(StatusCode::ACCEPTED)
}

/// `DELETE /connections/{id}`: closes one connection at once.
pub async fn close(
  State(state): State<Arc<ApiState>>,
  ConnectInfo(from): ConnectInfo<SocketAddr>,
  Path(conn_id): Path<u64>,
) -> Result<StatusCode, ApiError> {
  if !state.actions.close(conn_id).await {
    return Err(no_connection(conn_id));
  }
  info!("API action from {from}: closed connection {conn_id}");
  Ok(StatusCode::NO_CONTENT)
}

/// `POST /drain?timeout_ms=`: refuses new requests and tells every client to move.
pub async fn drain(
  State(state): State<Arc<ApiState>>,
  ConnectInfo(from): ConnectInfo<SocketAddr>,
  query: Result<Query<TimeoutQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
  let timeout_ms = timeout(query)?;
  let told = state.actions.drain(timeout_ms).await;
  info!("API action from {from}: drain, GOAWAY to {told} connection(s), timeout {timeout_ms} ms");
  Ok(Json(json!({ "told": told, "timeout_ms": timeout_ms })))
}

#[derive(Debug, Deserialize)]
pub struct RedirectBody {
  /// `null` or empty: GOAWAY carries no URI.
  uri: Option<String>,
}

pub async fn redirect_uri(State(state): State<Arc<ApiState>>) -> Json<Value> {
  Json(json!({ "uri": state.actions.redirect_uri().await }))
}

/// `PUT /config/redirect_uri` with `{"uri": "..."}`.
pub async fn set_redirect_uri(
  State(state): State<Arc<ApiState>>,
  ConnectInfo(from): ConnectInfo<SocketAddr>,
  body: Result<Json<RedirectBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
  let uri = parse_body(body)?.uri.filter(|u| !u.is_empty());
  if uri.as_ref().is_some_and(|u| u.len() > MAX_URI_BYTES) {
    return Err(
      ApiError::new(StatusCode::BAD_REQUEST, "bad_body")
        .detail(format!("uri is at most {MAX_URI_BYTES} bytes")),
    );
  }
  info!("API action from {from}: GOAWAY redirect URI set to {uri:?}");
  state.actions.set_redirect_uri(uri.clone()).await;
  Ok(Json(json!({ "uri": uri })))
}

#[derive(Debug, Deserialize)]
pub struct LoggingBody {
  /// Directives as in `RUST_LOG`, such as `info,relay=debug`.
  filter: String,
}

pub async fn logging(State(state): State<Arc<ApiState>>) -> Json<Value> {
  Json(json!({ "filter": state.actions.log_filter() }))
}

/// `PUT /logging` with `{"filter": "info,relay=debug"}`.
pub async fn set_logging(
  State(state): State<Arc<ApiState>>,
  ConnectInfo(from): ConnectInfo<SocketAddr>,
  body: Result<Json<LoggingBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
  let filter = parse_body(body)?.filter;
  state
    .actions
    .set_log_filter(&filter)
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "bad_filter").detail(e))?;
  info!("API action from {from}: log filter set to {filter:?}");
  Ok(Json(json!({ "filter": state.actions.log_filter() })))
}
