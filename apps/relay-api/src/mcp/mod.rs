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
//! The protocol itself is handled by `rmcp`: the 2026-07-28 revision statelessly,
//! and the earlier revisions, which start with `initialize`, with a session each. This module defines the tools. They call the same handlers
//! as the HTTP endpoints, so they read the relay through the same one-second
//! snapshots. The token and Origin checks run before a request reaches here, in
//! `http::access`.

pub mod token;

use crate::ApiState;
use crate::config::Feature;
use crate::http::error::ApiError;
use axum::extract::{Path, Query, State};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::info;

/// The most items a list tool returns, so one call cannot flood the agent.
const MAX_ITEMS: u64 = 100;
const MAX_TIMELINE_ENTRIES: u64 = 200;
/// How long clients get to move after GOAWAY when a call does not say.
const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
const MAX_URI_BYTES: usize = 8192;

/// What follows the relay's introduction in the instructions.
const INSTRUCTIONS: &str = "Start with relay_overview. list_connections and get_connection show who is \
connected and what they publish and subscribe; connection_timeline shows the control \
messages a connection exchanged, with each request next to its response, which is \
where to look when a peer's request failed. list_tracks and get_track show tracks, \
their fan-out and cache. latency_report and slowlog show object processing time. \
Connection ids, track ids and request ids are numbers taken from these results.";

/// Each tool, the feature it belongs to, and whether it changes the relay. A tool
/// is offered only when its feature is on, and a write tool only with
/// `api.mcp_write` as well.
const TOOLS: [(&str, Feature, bool); 17] = [
  ("relay_overview", Feature::State, false),
  ("list_connections", Feature::State, false),
  ("get_connection", Feature::State, false),
  ("connection_timeline", Feature::Qlog, false),
  ("list_tracks", Feature::State, false),
  ("get_track", Feature::State, false),
  ("namespaces", Feature::State, false),
  ("upstreams", Feature::State, false),
  ("zones", Feature::State, false),
  ("cluster", Feature::State, false),
  ("latency_report", Feature::Latency, false),
  ("slowlog", Feature::Latency, false),
  ("send_goaway", Feature::Actions, true),
  ("close_connection", Feature::Actions, true),
  ("drain_relay", Feature::Actions, true),
  ("set_log_filter", Feature::Actions, true),
  ("set_redirect_uri", Feature::Actions, true),
];

/// The HTTP service for `/api/v1/mcp`.
pub fn service(state: Arc<ApiState>) -> StreamableHttpService<McpServer, LocalSessionManager> {
  // A loopback API checks the Host header against DNS rebinding; on a network
  // address the client list is set by api.allow and the token instead.
  // 2026-07-28 requests are served statelessly whatever this says; clients of
  // the 2025 revisions start with initialize, which needs a session.
  let mut config = StreamableHttpServerConfig::default().with_legacy_session_mode(true);
  config.json_response = true;
  if !state.config.bind.ip().is_loopback() {
    config = config.with_allowed_hosts(Vec::<String>::new());
  }
  StreamableHttpService::new(
    move || Ok(McpServer::new(state.clone())),
    Default::default(),
    config,
  )
}

#[derive(Clone)]
pub struct McpServer {
  state: Arc<ApiState>,
  tool_router: ToolRouter<Self>,
}

impl McpServer {
  fn new(state: Arc<ApiState>) -> Self {
    let mut tool_router = Self::tool_router();
    for (name, feature, write) in TOOLS {
      if !offered(&state, feature, write) {
        tool_router.remove_route(name);
      }
    }
    McpServer { state, tool_router }
  }
}

fn offered(state: &ApiState, feature: Feature, write: bool) -> bool {
  let feature_on = match feature {
    // The timeline is read from the trace files.
    Feature::Qlog => state.config.has(Feature::Qlog) && state.config.qlog_folder.is_some(),
    feature => state.config.has(feature),
  };
  feature_on && (!write || state.config.mcp_write)
}

/// The relay's name and where it serves MoQ, so an agent connected to several
/// relays can tell which one this is.
fn introduction(state: &ApiState) -> String {
  let config = state.inspect.config();
  let moq = config["bind"].as_str().unwrap_or("an unknown address");
  format!(
    "This server reports on the MOQtail relay named \"{}\", a Media over QUIC relay \
     serving MoQ on {moq}.",
    state.config.mcp_name
  )
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
  fn get_info(&self) -> ServerConfig {
    let mut server = Implementation::new("moqtail-relay", self.state.inspect.status().version);
    server.title = Some(format!("MOQtail relay {}", self.state.config.mcp_name));
    ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
      .with_server_info(server)
      .with_instructions(format!("{} {INSTRUCTIONS}", introduction(&self.state)))
  }
}

// Tool arguments. Each derives a JSON schema, which rmcp offers in tools/list.

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ConnectionsArgs {
  /// List recently closed connections instead of the open ones.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  closed: Option<bool>,
  #[serde(flatten)]
  list: ListArgs,
}

#[derive(Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ListArgs {
  /// A field to sort by; a leading - sorts in descending order.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  sort: Option<String>,
  /// At most 100.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  limit: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  offset: Option<u64>,
  /// Only entries whose namespace starts with this, such as live/room1.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  prefix: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ConnId {
  conn_id: u64,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TimelineArgs {
  conn_id: u64,
  /// Entries returned, at most 200.
  #[serde(default)]
  limit: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TrackId {
  track_id: u64,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WindowArgs {
  /// 10s, 60s (the default), 5m or start.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  window: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LimitArgs {
  /// At most 100.
  #[serde(default)]
  limit: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct GoawayArgs {
  conn_id: u64,
  /// Milliseconds the client gets to move before the relay closes it; 10000 when
  /// left out, at most 600000.
  #[serde(default)]
  timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TimeoutArgs {
  /// Milliseconds clients get to move before the relay closes them; 10000 when
  /// left out, at most 600000.
  #[serde(default)]
  timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct FilterArgs {
  /// Written as for RUST_LOG, such as info,relay=debug.
  filter: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UriArgs {
  /// Where GOAWAY sends clients; null or empty sends none.
  uri: Option<String>,
}

type ToolResult = Result<CallToolResult, ErrorData>;

/// A tool's answer. Structured content must be an object, so lists are wrapped.
fn ok(value: Value) -> ToolResult {
  let value = if value.is_object() {
    value
  } else {
    json!({ "items": value })
  };
  Ok(CallToolResult::structured(value))
}

/// A failed call is a result the agent can read and correct, not a protocol error.
fn fail(text: impl Into<String>) -> ToolResult {
  Ok(CallToolResult::error(vec![ContentBlock::text(text.into())]))
}

fn api_failure(error: ApiError) -> ToolResult {
  fail(serde_json::to_string(&error).unwrap_or_else(|_| "request failed".into()))
}

fn to_value(value: impl Serialize) -> ToolResult {
  match serde_json::to_value(value) {
    Ok(value) => ok(value),
    Err(e) => fail(e.to_string()),
  }
}

/// Tool arguments as the query type of the matching HTTP handler.
fn query<T: DeserializeOwned>(args: impl Serialize) -> Result<Query<T>, String> {
  serde_json::to_value(args)
    .and_then(serde_json::from_value)
    .map(Query)
    .map_err(|e| format!("invalid arguments: {e}"))
}

fn capped(limit: Option<u64>, max: u64) -> u64 {
  limit.unwrap_or(max.min(50)).clamp(1, max)
}

fn timeout(timeout_ms: Option<u64>) -> Result<u64, String> {
  let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
  if timeout_ms > MAX_TIMEOUT_MS {
    return Err(format!("timeout_ms is at most {MAX_TIMEOUT_MS}"));
  }
  Ok(timeout_ms)
}

#[tool_router]
impl McpServer {
  fn st(&self) -> State<Arc<ApiState>> {
    State(self.state.clone())
  }

  #[tool(
    description = "The relay's version, uptime, connection and track counts, request results and traffic totals.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn relay_overview(&self) -> ToolResult {
    to_value(crate::state::overview::overview(self.st()).await.0)
  }

  #[tool(
    description = "Connections with their remote address, implementation, state, QUIC round-trip time, subscription count and request results. Set closed to list recently closed connections and why they closed.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn list_connections(
    &self,
    Parameters(mut args): Parameters<ConnectionsArgs>,
  ) -> ToolResult {
    args.list.limit = Some(capped(args.list.limit, MAX_ITEMS));
    let query = match query(args) {
      Ok(q) => q,
      Err(e) => return fail(e),
    };
    match crate::state::connections::list(self.st(), Ok(query)).await {
      Ok(page) => to_value(page.0),
      Err(e) => api_failure(e),
    }
  }

  #[tool(
    description = "One connection in full: SETUP options, announced namespaces, published tracks, subscriptions with their progress, QUIC statistics and close reason.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn get_connection(&self, Parameters(ConnId { conn_id }): Parameters<ConnId>) -> ToolResult {
    match crate::state::connections::detail(self.st(), Path(conn_id)).await {
      Ok(detail) => ok(detail.0),
      Err(e) => api_failure(e),
    }
  }

  #[tool(
    description = "The control messages one connection exchanged, in order: each request (SUBSCRIBE, PUBLISH, FETCH and the rest) with the responses on its stream, data streams cut short and the close, parse errors with their bytes, and data streams counted per track. Use it to see why a request failed.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn connection_timeline(
    &self,
    Parameters(TimelineArgs { conn_id, limit }): Parameters<TimelineArgs>,
  ) -> ToolResult {
    let limit = capped(limit, MAX_TIMELINE_ENTRIES) as usize;
    let response =
      match crate::qlog::trace(self.st(), Path(conn_id), Ok(Query(Default::default()))).await {
        Ok(response) => response,
        Err(e) => return api_failure(e),
      };
    // Without `follow` the body is the whole file, already in memory.
    let body = match axum::body::to_bytes(response.into_body(), usize::MAX).await {
      Ok(body) => body,
      Err(e) => return fail(e.to_string()),
    };
    let events: Vec<Value> = body
      .split(|b| *b == b'\n')
      .filter_map(|line| {
        let line = line.strip_prefix(b"\x1e").unwrap_or(line);
        serde_json::from_slice(line).ok()
      })
      .collect();
    ok(crate::qlog::timeline::build(&events, limit))
  }

  #[tool(
    description = "Tracks with their origin, status, publishers, subscriber count, largest location, cached groups and processing time.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn list_tracks(&self, Parameters(mut args): Parameters<ListArgs>) -> ToolResult {
    args.limit = Some(capped(args.limit, MAX_ITEMS));
    let query = match query(args) {
      Ok(q) => q,
      Err(e) => return fail(e),
    };
    match crate::state::tracks::list(self.st(), Ok(query)).await {
      Ok(page) => to_value(page.0),
      Err(e) => api_failure(e),
    }
  }

  #[tool(
    description = "One track in full: its publishers, every subscription to it, and the groups in its cache.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn get_track(&self, Parameters(TrackId { track_id }): Parameters<TrackId>) -> ToolResult {
    match crate::state::tracks::detail(self.st(), Path(track_id)).await {
      Ok(detail) => ok(detail.0),
      Err(e) => api_failure(e),
    }
  }

  #[tool(
    description = "Announced namespaces with the connections announcing them, and the namespace subscriptions the relay holds.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn namespaces(&self) -> ToolResult {
    to_value(crate::state::overview::namespaces(self.st()).await.0)
  }

  #[tool(
    description = "Publishers and cluster peers the relay sends requests to, with their health, round-trip time, request results and last error.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn upstreams(&self) -> ToolResult {
    to_value(crate::state::overview::upstreams(self.st()).await.0)
  }

  #[tool(
    description = "Traffic and request results per accounting zone (namespace prefix). Empty when no zones are configured.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn zones(&self) -> ToolResult {
    to_value(crate::state::zones::list(self.st()).await.0)
  }

  #[tool(
    description = "This relay's cluster identity, the link to each peer it dials (up or down, since when, why it last dropped), and for each peer session the namespaces advertised to it with their hop paths.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn cluster(&self) -> ToolResult {
    to_value(crate::state::overview::cluster(self.st()).await.0)
  }

  #[tool(
    description = "Object processing time inside the relay (ingest, queue, write, total) and time to first object, as percentiles in milliseconds, with the slowest tracks and subscriptions.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn latency_report(&self, Parameters(args): Parameters<WindowArgs>) -> ToolResult {
    let query = match query(args) {
      Ok(q) => q,
      Err(e) => return fail(e),
    };
    match crate::latency::report(self.st(), Ok(query)).await {
      Ok(report) => to_value(report.0),
      Err(e) => api_failure(e),
    }
  }

  #[tool(
    description = "The slowest recent deliveries and first objects, newest first, with the track, connection and request of each.",
    annotations(read_only_hint = true, open_world_hint = false)
  )]
  async fn slowlog(&self, Parameters(LimitArgs { limit }): Parameters<LimitArgs>) -> ToolResult {
    let limit = capped(limit, MAX_ITEMS) as usize;
    let report = match crate::latency::slowlog(self.st()).await {
      Ok(report) => report.0,
      Err(e) => return api_failure(e),
    };
    let mut report = match serde_json::to_value(report) {
      Ok(v) => v,
      Err(e) => return fail(e.to_string()),
    };
    if let Some(entries) = report["entries"].as_array_mut() {
      let cut = entries.len() > limit;
      entries.truncate(limit);
      report["truncated"] = json!(cut);
    }
    ok(report)
  }

  #[tool(
    description = "Sends GOAWAY to one connection, asking the client to move to another session, and closes the connection if it is still open when the timeout ends.",
    annotations(
      read_only_hint = false,
      destructive_hint = true,
      open_world_hint = false
    )
  )]
  async fn send_goaway(
    &self,
    Parameters(GoawayArgs {
      conn_id,
      timeout_ms,
    }): Parameters<GoawayArgs>,
  ) -> ToolResult {
    let timeout_ms = match timeout(timeout_ms) {
      Ok(t) => t,
      Err(e) => return fail(e),
    };
    if !self.state.actions.goaway(conn_id, timeout_ms).await {
      return fail(format!("no open connection has id {conn_id}"));
    }
    info!("API action from MCP: GOAWAY to connection {conn_id}, timeout {timeout_ms} ms");
    ok(json!({ "sent": true }))
  }

  #[tool(
    description = "Closes one connection at once, without asking the client first.",
    annotations(
      read_only_hint = false,
      destructive_hint = true,
      open_world_hint = false
    )
  )]
  async fn close_connection(
    &self,
    Parameters(ConnId { conn_id }): Parameters<ConnId>,
  ) -> ToolResult {
    if !self.state.actions.close(conn_id).await {
      return fail(format!("no open connection has id {conn_id}"));
    }
    info!("API action from MCP: closed connection {conn_id}");
    ok(json!({ "closed": true }))
  }

  #[tool(
    description = "Makes the relay refuse new requests, sends GOAWAY to every connection, and closes those still open when the timeout ends. Lasts until the relay is restarted.",
    annotations(
      read_only_hint = false,
      destructive_hint = true,
      open_world_hint = false
    )
  )]
  async fn drain_relay(
    &self,
    Parameters(TimeoutArgs { timeout_ms }): Parameters<TimeoutArgs>,
  ) -> ToolResult {
    let timeout_ms = match timeout(timeout_ms) {
      Ok(t) => t,
      Err(e) => return fail(e),
    };
    let told = self.state.actions.drain(timeout_ms).await;
    info!("API action from MCP: drain, GOAWAY to {told} connection(s), timeout {timeout_ms} ms");
    ok(json!({ "told": told, "timeout_ms": timeout_ms }))
  }

  #[tool(
    description = "Replaces the relay's log filter, written as for RUST_LOG, such as info,relay=debug.",
    annotations(
      read_only_hint = false,
      destructive_hint = true,
      idempotent_hint = true,
      open_world_hint = false
    )
  )]
  async fn set_log_filter(
    &self,
    Parameters(FilterArgs { filter }): Parameters<FilterArgs>,
  ) -> ToolResult {
    if let Err(e) = self.state.actions.set_log_filter(&filter) {
      return fail(e);
    }
    info!("API action from MCP: log filter set to {filter:?}");
    ok(json!({ "filter": self.state.actions.log_filter() }))
  }

  #[tool(
    description = "Sets the URI that GOAWAY sends clients to; null or empty sends none.",
    annotations(
      read_only_hint = false,
      destructive_hint = true,
      idempotent_hint = true,
      open_world_hint = false
    )
  )]
  async fn set_redirect_uri(&self, Parameters(UriArgs { uri }): Parameters<UriArgs>) -> ToolResult {
    let uri = uri.filter(|u| !u.is_empty());
    if uri.as_ref().is_some_and(|u| u.len() > MAX_URI_BYTES) {
      return fail(format!("uri is at most {MAX_URI_BYTES} bytes"));
    }
    info!("API action from MCP: GOAWAY redirect URI set to {uri:?}");
    self.state.actions.set_redirect_uri(uri.clone()).await;
    ok(json!({ "uri": uri }))
  }
}
