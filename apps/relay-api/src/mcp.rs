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

//! `/api/v1/mcp`: a Model Context Protocol server, so an AI agent can ask the
//! relay what the dashboard shows.
//!
//! It is stateless and answers every request with one JSON body. It speaks two
//! protocol eras, told apart by the request: a 2026-07-28 request carries the
//! protocol version in `params._meta` and needs no handshake, while the 2025
//! revisions start with `initialize` and then send only the `MCP-Protocol-Version`
//! header. The tools call the same handlers as the HTTP endpoints, so they read
//! the relay through the same one-second snapshots.

use crate::ApiState;
use crate::config::Feature;
use crate::error::ApiError;
use crate::list::ListQuery;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::info;

/// Revisions without a handshake, newest first.
const MODERN: [&str; 1] = ["2026-07-28"];
/// Revisions that start with `initialize`, newest first.
const LEGACY: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
/// What a legacy request without the version header is read as, as the
/// specification allows for clients older than the header.
const HEADERLESS: &str = "2025-03-26";
const VERSION_HEADER: &str = "mcp-protocol-version";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// How long a client may keep the tool list: it changes only with the config.
const LIST_TTL_MS: u64 = 3_600_000;
/// The most items a list tool returns, so one call cannot flood the agent.
const MAX_ITEMS: usize = 100;
const MAX_TIMELINE_ENTRIES: usize = 200;

const INSTRUCTIONS: &str = "This server reports on one MOQtail relay, a Media over QUIC \
relay. Start with relay_overview. list_connections and get_connection show who is \
connected and what they publish and subscribe; connection_timeline shows the control \
messages a connection exchanged, with each request next to its response, which is \
where to look when a peer's request failed. list_tracks and get_track show tracks, \
their fan-out and cache. latency_report and slowlog show object processing time. \
Connection ids, track ids and request ids are numbers taken from these results.";

const JSONRPC_PARSE_ERROR: i64 = -32700;
const JSONRPC_INVALID_REQUEST: i64 = -32600;
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
const JSONRPC_INVALID_PARAMS: i64 = -32602;
const HEADER_MISMATCH: i64 = -32020;
const UNSUPPORTED_VERSION: i64 = -32022;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
  Modern,
  Legacy,
}

/// A JSON-RPC error with the HTTP status it goes out with.
struct RpcError {
  status: StatusCode,
  code: i64,
  message: String,
  data: Option<Value>,
}

impl RpcError {
  fn new(status: StatusCode, code: i64, message: impl Into<String>) -> Self {
    RpcError {
      status,
      code,
      message: message.into(),
      data: None,
    }
  }

  fn unsupported(requested: &str) -> Self {
    let supported: Vec<&str> = MODERN.iter().chain(LEGACY.iter()).copied().collect();
    RpcError {
      data: Some(json!({ "supported": supported, "requested": requested })),
      ..RpcError::new(
        StatusCode::BAD_REQUEST,
        UNSUPPORTED_VERSION,
        "Unsupported protocol version",
      )
    }
  }

  fn response(self, id: &Value) -> Response {
    let mut error = json!({ "code": self.code, "message": self.message });
    if let Some(data) = self.data {
      error["data"] = data;
    }
    (
      self.status,
      Json(json!({ "jsonrpc": "2.0", "id": id, "error": error })),
    )
      .into_response()
  }
}

pub async fn handle(
  State(state): State<Arc<ApiState>>,
  headers: HeaderMap,
  body: Bytes,
) -> Response {
  let message: Value = match serde_json::from_slice(&body) {
    Ok(message) => message,
    Err(e) => {
      return RpcError::new(
        StatusCode::BAD_REQUEST,
        JSONRPC_PARSE_ERROR,
        format!("Parse error: {e}"),
      )
      .response(&Value::Null);
    }
  };
  let Some(message) = message.as_object() else {
    return RpcError::new(
      StatusCode::BAD_REQUEST,
      JSONRPC_INVALID_REQUEST,
      "the body must be one JSON-RPC message; batches are not supported",
    )
    .response(&Value::Null);
  };
  let id = message.get("id").cloned();
  let Some(method) = message.get("method").and_then(Value::as_str) else {
    // A response to a server request; this server sends none, so there is
    // nothing to match it to.
    if id.is_some() && (message.contains_key("result") || message.contains_key("error")) {
      return StatusCode::ACCEPTED.into_response();
    }
    return RpcError::new(
      StatusCode::BAD_REQUEST,
      JSONRPC_INVALID_REQUEST,
      "no method",
    )
    .response(id.as_ref().unwrap_or(&Value::Null));
  };
  // A notification needs no answer; this server acts on none.
  let Some(id) = id else {
    return StatusCode::ACCEPTED.into_response();
  };
  let params = message.get("params").cloned().unwrap_or(json!({}));

  let era = match era(method, &params, &headers) {
    Ok(era) => era,
    Err(error) => return error.response(&id),
  };
  match answer(&state, era, method, &params).await {
    Ok(result) => Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response(),
    Err(error) => error.response(&id),
  }
}

/// Which era a request belongs to, after checking its version and headers.
fn era(method: &str, params: &Value, headers: &HeaderMap) -> Result<Era, RpcError> {
  if method == "initialize" {
    return Ok(Era::Legacy);
  }
  let header = headers.get(VERSION_HEADER).and_then(|v| v.to_str().ok());
  let meta = &params["_meta"];
  if let Some(version) = meta[META_VERSION].as_str() {
    if header != Some(version) {
      return Err(RpcError::new(
        StatusCode::BAD_REQUEST,
        HEADER_MISMATCH,
        "the MCP-Protocol-Version header must match the protocol version in _meta",
      ));
    }
    if !MODERN.contains(&version) {
      return Err(RpcError::unsupported(version));
    }
    if !meta[META_CAPABILITIES].is_object() {
      return Err(RpcError::new(
        StatusCode::BAD_REQUEST,
        JSONRPC_INVALID_PARAMS,
        format!("_meta must carry {META_CAPABILITIES}"),
      ));
    }
    return Ok(Era::Modern);
  }
  let version = header.unwrap_or(HEADERLESS);
  if MODERN.contains(&version) {
    return Err(RpcError::new(
      StatusCode::BAD_REQUEST,
      HEADER_MISMATCH,
      format!("a {version} request carries its protocol version in _meta"),
    ));
  }
  if !LEGACY.contains(&version) {
    return Err(RpcError::unsupported(version));
  }
  Ok(Era::Legacy)
}

fn server_info(state: &ApiState) -> Value {
  json!({ "name": "moqtail-relay", "version": state.inspect.status().version })
}

async fn answer(
  state: &Arc<ApiState>,
  era: Era,
  method: &str,
  params: &Value,
) -> Result<Value, RpcError> {
  let mut result = match (era, method) {
    (Era::Legacy, "initialize") => {
      let requested = params["protocolVersion"].as_str().unwrap_or("");
      // The client's revision when it is one of ours, otherwise our newest.
      let version = LEGACY
        .iter()
        .find(|v| **v == requested)
        .unwrap_or(&LEGACY[0]);
      json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": server_info(state),
        "instructions": INSTRUCTIONS,
      })
    }
    (Era::Legacy, "ping") => json!({}),
    (Era::Modern, "server/discover") => {
      let supported: Vec<&str> = MODERN.iter().chain(LEGACY.iter()).copied().collect();
      json!({
        "supportedVersions": supported,
        "capabilities": { "tools": {} },
        "instructions": INSTRUCTIONS,
        "ttlMs": LIST_TTL_MS,
        "cacheScope": "private",
        "_meta": { "io.modelcontextprotocol/serverInfo": server_info(state) },
      })
    }
    (_, "tools/list") => {
      let mut result = json!({ "tools": tools(state) });
      if era == Era::Modern {
        result["ttlMs"] = json!(LIST_TTL_MS);
        result["cacheScope"] = json!("private");
      }
      result
    }
    (_, "tools/call") => {
      let Some(name) = params["name"].as_str() else {
        return Err(RpcError::new(
          StatusCode::OK,
          JSONRPC_INVALID_PARAMS,
          "tools/call needs a tool name",
        ));
      };
      if !tools(state).iter().any(|t| t["name"] == name) {
        return Err(RpcError::new(
          StatusCode::OK,
          JSONRPC_INVALID_PARAMS,
          format!("no tool named {name}"),
        ));
      }
      let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
      match call(state, name, arguments).await {
        Ok(value) => {
          // Structured content must be an object; lists are wrapped in one.
          let structured = if value.is_object() {
            value
          } else {
            json!({ "items": value })
          };
          json!({
            "content": [{ "type": "text", "text": structured.to_string() }],
            "structuredContent": structured,
          })
        }
        // A failed call is a result the agent can read and correct, not a
        // protocol error.
        Err(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
      }
    }
    _ => {
      // The current revision answers an unknown method with 404.
      let status = if era == Era::Modern {
        StatusCode::NOT_FOUND
      } else {
        StatusCode::OK
      };
      return Err(RpcError::new(
        status,
        JSONRPC_METHOD_NOT_FOUND,
        format!("Method not found: {method}"),
      ));
    }
  };
  if era == Era::Modern {
    result["resultType"] = json!("complete");
  }
  Ok(result)
}

/// One tool: its name, what it is for, its arguments, and when it is offered.
struct Tool {
  name: &'static str,
  description: &'static str,
  arguments: fn() -> Value,
  feature: Feature,
  /// Changes the relay; offered only with `api_mcp_write`.
  write: bool,
}

fn no_arguments() -> Value {
  json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn list_arguments() -> Value {
  json!({
    "type": "object",
    "properties": {
      "sort": { "type": "string", "description": "A field to sort by; a leading - sorts in descending order." },
      "limit": { "type": "integer", "minimum": 1, "maximum": MAX_ITEMS },
      "offset": { "type": "integer", "minimum": 0 },
      "prefix": { "type": "string", "description": "Only entries whose namespace starts with this, such as live/room1." },
    },
    "additionalProperties": false,
  })
}

fn conn_id_arguments() -> Value {
  json!({
    "type": "object",
    "properties": { "conn_id": { "type": "integer", "minimum": 0 } },
    "required": ["conn_id"],
    "additionalProperties": false,
  })
}

fn timeout_property() -> Value {
  json!({ "type": "integer", "minimum": 0, "maximum": 600000, "description": "How long clients get to move, in milliseconds; 10000 when left out." })
}

const TOOLS: &[Tool] = &[
  Tool {
    name: "relay_overview",
    description: "The relay's version, uptime, connection and track counts, request results and traffic totals.",
    arguments: no_arguments,
    feature: Feature::State,
    write: false,
  },
  Tool {
    name: "list_connections",
    description: "Connections with their remote address, implementation, state, QUIC round-trip time, subscription count and request results. Set closed to list recently closed connections and why they closed.",
    arguments: || {
      let mut arguments = list_arguments();
      arguments["properties"]["closed"] = json!({ "type": "boolean" });
      arguments
    },
    feature: Feature::State,
    write: false,
  },
  Tool {
    name: "get_connection",
    description: "One connection in full: SETUP options, announced namespaces, published tracks, subscriptions with their progress, QUIC statistics and close reason.",
    arguments: conn_id_arguments,
    feature: Feature::State,
    write: false,
  },
  Tool {
    name: "connection_timeline",
    description: "The control messages one connection exchanged, in order: each request (SUBSCRIBE, PUBLISH, FETCH and the rest) with the responses on its stream, parse errors with their bytes, and data streams counted per track. Use it to see why a request failed.",
    arguments: || {
      let mut arguments = conn_id_arguments();
      arguments["properties"]["limit"] =
        json!({ "type": "integer", "minimum": 1, "maximum": MAX_TIMELINE_ENTRIES });
      arguments
    },
    feature: Feature::Qlog,
    write: false,
  },
  Tool {
    name: "list_tracks",
    description: "Tracks with their origin, status, publishers, subscriber count, largest location, cached groups and processing time.",
    arguments: list_arguments,
    feature: Feature::State,
    write: false,
  },
  Tool {
    name: "get_track",
    description: "One track in full: its publishers, every subscription to it, and the groups in its cache.",
    arguments: || {
      json!({
        "type": "object",
        "properties": { "track_id": { "type": "integer", "minimum": 0 } },
        "required": ["track_id"],
        "additionalProperties": false,
      })
    },
    feature: Feature::State,
    write: false,
  },
  Tool {
    name: "namespaces",
    description: "Announced namespaces with the connections announcing them, and the namespace subscriptions the relay holds.",
    arguments: no_arguments,
    feature: Feature::State,
    write: false,
  },
  Tool {
    name: "upstreams",
    description: "Publishers and cluster peers the relay sends requests to, with their health, round-trip time, request results and last error.",
    arguments: no_arguments,
    feature: Feature::State,
    write: false,
  },
  Tool {
    name: "zones",
    description: "Traffic and request results per accounting zone (namespace prefix). Empty when no zones are configured.",
    arguments: no_arguments,
    feature: Feature::State,
    write: false,
  },
  Tool {
    name: "cluster",
    description: "This relay's cluster identity, its peers and the namespaces they advertise.",
    arguments: no_arguments,
    feature: Feature::State,
    write: false,
  },
  Tool {
    name: "latency_report",
    description: "Object processing time inside the relay (ingest, queue, write, total) and time to first object, as percentiles in milliseconds, with the slowest tracks and subscriptions.",
    arguments: || {
      json!({
        "type": "object",
        "properties": { "window": { "type": "string", "enum": ["10s", "60s", "5m", "start"] } },
        "additionalProperties": false,
      })
    },
    feature: Feature::Latency,
    write: false,
  },
  Tool {
    name: "slowlog",
    description: "The slowest recent deliveries and first objects, newest first, with the track, connection and request of each.",
    arguments: || {
      json!({
        "type": "object",
        "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": MAX_ITEMS } },
        "additionalProperties": false,
      })
    },
    feature: Feature::Latency,
    write: false,
  },
  Tool {
    name: "send_goaway",
    description: "Sends GOAWAY to one connection, asking the client to move to another session.",
    arguments: || {
      let mut arguments = conn_id_arguments();
      arguments["properties"]["timeout_ms"] = timeout_property();
      arguments
    },
    feature: Feature::Actions,
    write: true,
  },
  Tool {
    name: "close_connection",
    description: "Closes one connection at once, without asking the client first.",
    arguments: conn_id_arguments,
    feature: Feature::Actions,
    write: true,
  },
  Tool {
    name: "drain_relay",
    description: "Makes the relay refuse new requests and sends GOAWAY to every connection. Lasts until the relay is restarted.",
    arguments: || {
      json!({
        "type": "object",
        "properties": { "timeout_ms": timeout_property() },
        "additionalProperties": false,
      })
    },
    feature: Feature::Actions,
    write: true,
  },
  Tool {
    name: "set_log_filter",
    description: "Replaces the relay's log filter, written as for RUST_LOG, such as info,relay=debug.",
    arguments: || {
      json!({
        "type": "object",
        "properties": { "filter": { "type": "string" } },
        "required": ["filter"],
        "additionalProperties": false,
      })
    },
    feature: Feature::Actions,
    write: true,
  },
  Tool {
    name: "set_redirect_uri",
    description: "Sets the URI that GOAWAY sends clients to; null or empty sends none.",
    arguments: || {
      json!({
        "type": "object",
        "properties": { "uri": { "type": ["string", "null"] } },
        "required": ["uri"],
        "additionalProperties": false,
      })
    },
    feature: Feature::Actions,
    write: true,
  },
];

fn offered(state: &ApiState, tool: &Tool) -> bool {
  let feature_on = match tool.feature {
    // The timeline is read from the trace files.
    Feature::Qlog => state.config.has(Feature::Qlog) && state.config.qlog_folder.is_some(),
    feature => state.config.has(feature),
  };
  feature_on && (!tool.write || state.config.mcp_write)
}

/// The tools on offer, in a fixed order.
fn tools(state: &ApiState) -> Vec<Value> {
  TOOLS
    .iter()
    .filter(|t| offered(state, t))
    .map(|t| {
      let annotations = if t.write {
        json!({ "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false })
      } else {
        json!({ "readOnlyHint": true, "openWorldHint": false })
      };
      json!({
        "name": t.name,
        "description": t.description,
        "inputSchema": (t.arguments)(),
        "outputSchema": { "type": "object" },
        "annotations": annotations,
      })
    })
    .collect()
}

fn arguments<T: DeserializeOwned>(arguments: Value) -> Result<T, String> {
  serde_json::from_value(arguments).map_err(|e| format!("invalid arguments: {e}"))
}

fn conn_id(arguments: &Value) -> Result<u64, String> {
  arguments["conn_id"]
    .as_u64()
    .ok_or_else(|| "conn_id must be a non-negative integer".to_string())
}

fn timeout_ms(arguments: &Value) -> Result<u64, String> {
  match &arguments["timeout_ms"] {
    Value::Null => Ok(10_000),
    v => v
      .as_u64()
      .filter(|t| *t <= 600_000)
      .ok_or_else(|| "timeout_ms must be an integer from 0 to 600000".to_string()),
  }
}

/// Caps a list tool's `limit` at [`MAX_ITEMS`].
fn capped(mut arguments: Value) -> Value {
  if let Some(object) = arguments.as_object_mut() {
    let limit = object.get("limit").and_then(Value::as_u64).unwrap_or(50);
    object.insert("limit".into(), json!(limit.clamp(1, MAX_ITEMS as u64)));
  }
  arguments
}

fn to_value(value: impl Serialize) -> Result<Value, String> {
  serde_json::to_value(value).map_err(|e| e.to_string())
}

fn api_error(error: ApiError) -> String {
  serde_json::to_string(&error).unwrap_or_else(|_| "request failed".into())
}

async fn call(state: &Arc<ApiState>, name: &str, args: Value) -> Result<Value, String> {
  let st = || State(state.clone());
  match name {
    "relay_overview" => to_value(crate::overview::overview(st()).await.0),
    "list_connections" => {
      let query = arguments(capped(args))?;
      let page = crate::connections::list(st(), Ok(Query(query)))
        .await
        .map_err(api_error)?;
      to_value(page.0)
    }
    "get_connection" => {
      let id = conn_id(&args)?;
      to_value(
        crate::connections::detail(st(), Path(id))
          .await
          .map_err(api_error)?
          .0,
      )
    }
    "connection_timeline" => {
      let id = conn_id(&args)?;
      let limit = args["limit"]
        .as_u64()
        .unwrap_or(MAX_TIMELINE_ENTRIES as u64);
      let limit = limit.clamp(1, MAX_TIMELINE_ENTRIES as u64) as usize;
      let response = crate::qlog::trace(st(), Path(id), Ok(Query(Default::default())))
        .await
        .map_err(api_error)?;
      // Without `follow` the body is the whole file, already in memory.
      let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|e| e.to_string())?;
      let events: Vec<Value> = body
        .split(|b| *b == b'\n')
        .filter_map(|line| {
          let line = line.strip_prefix(b"\x1e").unwrap_or(line);
          serde_json::from_slice(line).ok()
        })
        .collect();
      Ok(crate::timeline::build(&events, limit))
    }
    "list_tracks" => {
      let query: ListQuery = arguments(capped(args))?;
      to_value(
        crate::tracks::list(st(), Ok(Query(query)))
          .await
          .map_err(api_error)?
          .0,
      )
    }
    "get_track" => {
      let id = args["track_id"]
        .as_u64()
        .ok_or("track_id must be a non-negative integer")?;
      to_value(
        crate::tracks::detail(st(), Path(id))
          .await
          .map_err(api_error)?
          .0,
      )
    }
    "namespaces" => to_value(crate::overview::namespaces(st()).await.0),
    "upstreams" => to_value(crate::overview::upstreams(st()).await.0),
    "zones" => to_value(crate::zones::list(st()).await.0),
    "cluster" => to_value(crate::overview::cluster(st()).await.0),
    "latency_report" => {
      let query = arguments(args)?;
      to_value(
        crate::latency::report(st(), Ok(Query(query)))
          .await
          .map_err(api_error)?
          .0,
      )
    }
    "slowlog" => {
      let limit = args["limit"]
        .as_u64()
        .unwrap_or(MAX_ITEMS as u64)
        .clamp(1, MAX_ITEMS as u64);
      let mut report = to_value(crate::latency::slowlog(st()).await.map_err(api_error)?.0)?;
      if let Some(entries) = report["entries"].as_array_mut() {
        let cut = entries.len() > limit as usize;
        entries.truncate(limit as usize);
        report["truncated"] = json!(cut);
      }
      Ok(report)
    }
    "send_goaway" => {
      let (id, timeout) = (conn_id(&args)?, timeout_ms(&args)?);
      if !state.actions.goaway(id, timeout).await {
        return Err(format!("no open connection has id {id}"));
      }
      info!("API action from MCP: GOAWAY to connection {id}, timeout {timeout} ms");
      Ok(json!({ "sent": true }))
    }
    "close_connection" => {
      let id = conn_id(&args)?;
      if !state.actions.close(id).await {
        return Err(format!("no open connection has id {id}"));
      }
      info!("API action from MCP: closed connection {id}");
      Ok(json!({ "closed": true }))
    }
    "drain_relay" => {
      let timeout = timeout_ms(&args)?;
      let told = state.actions.drain(timeout).await;
      info!("API action from MCP: drain, GOAWAY to {told} connection(s), timeout {timeout} ms");
      Ok(json!({ "told": told, "timeout_ms": timeout }))
    }
    "set_log_filter" => {
      let filter = args["filter"].as_str().ok_or("filter must be a string")?;
      state.actions.set_log_filter(filter)?;
      info!("API action from MCP: log filter set to {filter:?}");
      Ok(json!({ "filter": state.actions.log_filter() }))
    }
    "set_redirect_uri" => {
      let uri = match &args["uri"] {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        Value::String(s) if s.len() <= 8192 => Some(s.clone()),
        _ => return Err("uri must be null or a string of at most 8192 bytes".into()),
      };
      info!("API action from MCP: GOAWAY redirect URI set to {uri:?}");
      state.actions.set_redirect_uri(uri.clone()).await;
      Ok(json!({ "uri": uri }))
    }
    _ => Err(format!("no tool named {name}")),
  }
}
