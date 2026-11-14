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

//! The `mcp` feature: the MCP server, its token and the tools it offers.

mod common;

use common::*;

const MCP_TOKEN: &str = "mcp_test_token";

fn mcp_client(write: bool) -> Client {
  Client::new(ApiSettings {
    features: Some(vec![
      "state".into(),
      "latency".into(),
      "actions".into(),
      "mcp".into(),
    ]),
    read_token: Some("r3ad".into()),
    write_token: Some("wr1te".into()),
    mcp_token_hash: Some(relay_api::token::TokenHash::new(MCP_TOKEN).to_string()),
    mcp_write: write,
    mcp_name: Some("interop-paris".into()),
    ..Default::default()
  })
}

/// One MCP exchange as a client sends it.
struct Mcp<'a> {
  version: Option<&'a str>,
  session: Option<&'a str>,
}

impl Client {
  /// Sends one MCP message with the MCP token and the headers an MCP client sends,
  /// and returns the status, the JSON-RPC answer and the response headers. The
  /// answer may come as JSON or as one server-sent event.
  async fn mcp(&self, how: Mcp<'_>, message: Value) -> (StatusCode, Value, axum::http::HeaderMap) {
    let auth = format!("Bearer {MCP_TOKEN}");
    let method = message["method"].as_str().unwrap_or("").to_string();
    let mut headers = vec![
      ("authorization", auth.as_str()),
      ("host", "127.0.0.1:4480"),
      ("accept", "application/json, text/event-stream"),
    ];
    if let Some(version) = how.version {
      headers.push(("mcp-protocol-version", version));
      // The current revision names the method in a header too.
      if version == "2026-07-28" {
        headers.push(("mcp-method", method.as_str()));
      }
    }
    let tool = message["params"]["name"].as_str().unwrap_or("").to_string();
    if how.version == Some("2026-07-28") && method == "tools/call" {
      headers.push(("mcp-name", tool.as_str()));
    }
    if let Some(session) = how.session {
      headers.push(("mcp-session-id", session));
    }
    let (status, body, response_headers) = self
      .send_json(
        Method::POST,
        "/api/v1/mcp",
        "127.0.0.1:5000",
        &headers,
        Some(message),
      )
      .await;
    let body = match &body {
      Value::String(text) => text
        .lines()
        // The first data line of a stream can be an empty priming event.
        .filter_map(|l| l.strip_prefix("data:"))
        .find_map(|data| serde_json::from_str(data.trim()).ok())
        .unwrap_or(body),
      _ => body,
    };
    (status, body, response_headers)
  }
}

/// A 2026-07-28 request, with the version in `_meta`.
fn modern(method: &str, params: Value) -> Value {
  let mut params = params;
  params["_meta"] = json!({
    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
    "io.modelcontextprotocol/clientCapabilities": {},
  });
  json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

fn legacy(method: &str, params: Value) -> Value {
  json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

const MODERN: Mcp = Mcp {
  version: Some("2026-07-28"),
  session: None,
};

#[tokio::test]
async fn mcp_takes_only_its_own_token_and_refuses_foreign_origins() {
  let client = mcp_client(false);
  let local = "127.0.0.1:5000";
  let list = modern("tools/list", json!({}));
  let (status, ..) = client
    .send_json(Method::POST, "/api/v1/mcp", local, &[], Some(list.clone()))
    .await;
  assert_eq!(status, StatusCode::UNAUTHORIZED);
  for token in ["Bearer r3ad", "Bearer wr1te"] {
    let (status, ..) = client
      .send_json(
        Method::POST,
        "/api/v1/mcp",
        local,
        &[("authorization", token)],
        Some(list.clone()),
      )
      .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{token} does not open MCP");
  }
  let mcp = format!("Bearer {MCP_TOKEN}");
  let (status, ..) = client
    .send(
      Method::GET,
      "/api/v1/config",
      local,
      &[("authorization", &mcp)],
    )
    .await;
  assert_eq!(
    status,
    StatusCode::FORBIDDEN,
    "the MCP token opens nothing else"
  );
  let (status, ..) = client
    .send_json(
      Method::POST,
      "/api/v1/mcp",
      local,
      &[("authorization", &mcp), ("origin", "https://evil.example")],
      Some(list.clone()),
    )
    .await;
  assert_eq!(status, StatusCode::FORBIDDEN);
  assert_eq!(client.mcp(MODERN, list).await.0, StatusCode::OK);
}

#[tokio::test]
async fn mcp_serves_2025_clients_after_initialize() {
  let client = mcp_client(false);
  let init = legacy(
    "initialize",
    json!({
      "protocolVersion": "2025-11-25",
      "capabilities": {},
      "clientInfo": { "name": "test", "version": "1" },
    }),
  );
  let (status, body, headers) = client
    .mcp(
      Mcp {
        version: None,
        session: None,
      },
      init,
    )
    .await;
  assert_eq!(status, StatusCode::OK, "{body}");
  let result = &body["result"];
  assert_eq!(result["protocolVersion"], "2025-11-25");
  assert_eq!(result["serverInfo"]["name"], "moqtail-relay");
  // The relay introduces itself, so an agent with several relays can tell which
  // one this is.
  assert_eq!(result["serverInfo"]["title"], "MOQtail relay interop-paris");
  let instructions = result["instructions"].as_str().unwrap();
  assert!(instructions.contains("\"interop-paris\"") && instructions.contains("0.0.0.0:4433"));

  let session = headers
    .get("mcp-session-id")
    .and_then(|v| v.to_str().ok())
    .expect("a 2025 client gets a session")
    .to_string();
  let how = || Mcp {
    version: Some("2025-11-25"),
    session: Some(&session),
  };
  let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
  assert_eq!(client.mcp(how(), initialized).await.0, StatusCode::ACCEPTED);

  let (_, body, _) = client.mcp(how(), legacy("tools/list", json!({}))).await;
  let tools = body["result"]["tools"].as_array().unwrap();
  let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
  assert!(names.contains(&"relay_overview") && names.contains(&"get_connection"));
  assert!(
    !names.contains(&"send_goaway"),
    "write tools need api.mcp_write"
  );
  assert!(
    !names.contains(&"connection_timeline"),
    "no qlog folder, no timeline"
  );
  assert!(
    tools
      .iter()
      .all(|t| t["annotations"]["readOnlyHint"] == true)
  );

  let call = legacy(
    "tools/call",
    json!({ "name": "get_connection", "arguments": { "conn_id": 7 } }),
  );
  let (_, body, _) = client.mcp(how(), call).await;
  assert_eq!(body["result"]["structuredContent"]["conn_id"], 7);
  let text = body["result"]["content"][0]["text"].as_str().unwrap();
  assert_eq!(
    serde_json::from_str::<Value>(text).unwrap(),
    body["result"]["structuredContent"]
  );

  // A connection that is not open is a tool error the agent can read.
  let call = legacy(
    "tools/call",
    json!({ "name": "get_connection", "arguments": { "conn_id": 999 } }),
  );
  let (_, body, _) = client.mcp(how(), call).await;
  assert_eq!(body["result"]["isError"], true);

  // A tool this relay does not offer is refused.
  let call = legacy(
    "tools/call",
    json!({ "name": "send_goaway", "arguments": { "conn_id": 7 } }),
  );
  let (_, body, _) = client.mcp(how(), call).await;
  assert!(body["error"].is_object() || body["result"]["isError"] == true);
}

#[tokio::test]
async fn mcp_serves_2026_clients_without_a_handshake() {
  let client = mcp_client(true);
  let (status, body, headers) = client
    .mcp(MODERN, modern("server/discover", json!({})))
    .await;
  assert_eq!(status, StatusCode::OK, "{body}");
  assert!(
    headers.get("mcp-session-id").is_none(),
    "2026-07-28 is stateless"
  );
  let result = &body["result"];
  assert!(
    result["supportedVersions"]
      .as_array()
      .unwrap()
      .contains(&json!("2026-07-28"))
  );
  assert!(
    result["instructions"]
      .as_str()
      .unwrap()
      .contains("interop-paris")
  );

  let (_, body, _) = client.mcp(MODERN, modern("tools/list", json!({}))).await;
  let tools = body["result"]["tools"].as_array().unwrap();
  let goaway = tools.iter().find(|t| t["name"] == "send_goaway").unwrap();
  assert_eq!(goaway["annotations"]["destructiveHint"], true);

  let call = modern(
    "tools/call",
    json!({ "name": "send_goaway", "arguments": { "conn_id": 7, "timeout_ms": 500 } }),
  );
  let (_, body, _) = client.mcp(MODERN, call).await;
  assert_eq!(body["result"]["structuredContent"]["sent"], true, "{body}");
  let call = modern(
    "tools/call",
    json!({ "name": "send_goaway", "arguments": { "conn_id": 7, "timeout_ms": 9_999_999 } }),
  );
  let (_, body, _) = client.mcp(MODERN, call).await;
  assert_eq!(body["result"]["isError"], true, "the timeout is capped");
  assert_eq!(*client.relay.actions.lock().unwrap(), ["goaway 7 500"]);
}

#[test]
fn mcp_needs_a_token_hash_to_start() {
  let error = ApiConfig::from_settings(
    ApiSettings {
      features: Some(vec!["mcp".into()]),
      ..Default::default()
    },
    RelaySettings::default(),
  )
  .unwrap_err();
  assert!(error.to_string().contains("api.mcp_token_hash"));
  let error = ApiConfig::from_settings(
    ApiSettings {
      features: Some(vec!["mcp".into()]),
      mcp_token_hash: Some("plain-token".into()),
      ..Default::default()
    },
    RelaySettings::default(),
  )
  .unwrap_err();
  assert!(format!("{error:#}").contains("sha256d"));
  let (_, warnings) = ApiConfig::from_settings(
    ApiSettings {
      features: Some(vec!["mcp".into()]),
      mcp_token_hash: Some(relay_api::token::TokenHash::new("t").to_string()),
      mcp_write: true,
      ..Default::default()
    },
    RelaySettings::default(),
  )
  .unwrap();
  assert!(warnings.iter().any(|w| w.contains("api.mcp_write")));
}

#[test]
fn the_relay_is_named_after_its_host_unless_told_otherwise() {
  let (config, _) =
    ApiConfig::from_settings(ApiSettings::default(), RelaySettings::default()).unwrap();
  assert!(!config.mcp_name.is_empty() && config.mcp_name != "unnamed");
  let (config, _) = ApiConfig::from_settings(
    ApiSettings {
      mcp_name: Some("  ".into()),
      ..Default::default()
    },
    RelaySettings::default(),
  )
  .unwrap();
  assert_ne!(
    config.mcp_name.trim(),
    "",
    "a blank name falls back to the host name"
  );
}
