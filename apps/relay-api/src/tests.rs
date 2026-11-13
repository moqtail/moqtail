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

//! Request-level tests: each one sends a request through the full router,
//! including the access checks, as a client at a given address would.

use crate::config::{ApiConfig, ApiSettings};
use crate::{ApiState, routes};
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use moqtail_probe::{
  BoxFuture, CloseInfo, ConnectionInfo, ConnectionState, Inspect, Probe, QuicStats, RelayStatus,
  Role,
};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tower::ServiceExt;

/// A relay stand-in with settable lifecycle flags.
#[derive(Default)]
pub(crate) struct FakeRelay {
  pub accepting: AtomicBool,
  pub draining: AtomicBool,
}

impl Inspect for FakeRelay {
  fn status(&self) -> RelayStatus {
    RelayStatus {
      version: "moqtail/test".into(),
      pid: 1,
      started_at_ms: 0,
      accepting: self.accepting.load(Ordering::Relaxed),
      draining: self.draining.load(Ordering::Relaxed),
    }
  }

  fn config(&self) -> Value {
    json!({ "cache_size": 1000 })
  }

  fn connections(&self) -> BoxFuture<'_, Vec<ConnectionInfo>> {
    Box::pin(async {
      vec![
        connection(7, "live/room1", 10.0, 500),
        connection(9, "vod/a", 2.0, 9000),
      ]
    })
  }
}

/// A connection that announced `namespace`, with the given RTT and bytes sent.
pub(crate) fn connection(conn_id: u64, namespace: &str, rtt_ms: f64, sent: u64) -> ConnectionInfo {
  ConnectionInfo {
    conn_id,
    state: ConnectionState::Active,
    transport: "quic".into(),
    remote: "192.0.2.10:5000".into(),
    role: Role::Client,
    connected_at_ms: conn_id * 1000,
    implementation: Some("other-relay/1.0".into()),
    setup_options: vec![],
    announced_namespaces: vec![namespace.into()],
    published_tracks: vec![],
    subscriptions: vec![],
    quic: QuicStats {
      rtt_ms,
      sent_bytes: sent,
      ..Default::default()
    },
    close: None,
  }
}

pub(crate) struct Client {
  state: Arc<ApiState>,
  pub relay: Arc<FakeRelay>,
}

impl Client {
  pub fn new(settings: ApiSettings) -> Self {
    let (config, _) = ApiConfig::from_settings(settings).unwrap();
    let relay = Arc::new(FakeRelay::default());
    relay.accepting.store(true, Ordering::Relaxed);
    let state = Arc::new(ApiState::new(config, relay.clone()));
    Client { state, relay }
  }

  /// Sends a request from `from`, with optional extra headers, and returns the
  /// status and the parsed JSON body (`Null` when the body is empty).
  pub async fn send(
    &self,
    method: Method,
    path: &str,
    from: &str,
    headers: &[(&str, &str)],
  ) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
      request = request.header(*name, *value);
    }
    let mut request = request.body(Body::empty()).unwrap();
    let peer: SocketAddr = from.parse().unwrap();
    request.extensions_mut().insert(ConnectInfo(peer));
    let response = routes::router(self.state.clone())
      .oneshot(request)
      .await
      .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
      Value::Null
    } else {
      serde_json::from_slice(&bytes).unwrap()
    };
    (status, body, headers)
  }

  pub fn probe(&self) -> &dyn Probe {
    self.state.probe.as_ref()
  }

  pub async fn get(&self, path: &str) -> (StatusCode, Value) {
    let (status, body, _) = self.send(Method::GET, path, "127.0.0.1:5000", &[]).await;
    (status, body)
  }
}

#[tokio::test]
async fn index_lists_endpoints_with_their_features() {
  let client = Client::new(ApiSettings {
    features: Some(vec!["health".into()]),
    ..Default::default()
  });
  let (status, body) = client.get("/api/v1").await;
  assert_eq!(status, StatusCode::OK);
  assert_eq!(body["features"], json!(["health"]));
  let config = body["endpoints"]
    .as_array()
    .unwrap()
    .iter()
    .find(|e| e["path"] == "/api/v1/config")
    .unwrap();
  assert_eq!(config["feature"], "state");
  assert_eq!(config["enabled"], false);
}

#[tokio::test]
async fn an_endpoint_whose_feature_is_off_answers_404_with_the_setting() {
  let client = Client::new(ApiSettings {
    features: Some(vec!["health".into()]),
    ..Default::default()
  });
  let (status, body) = client.get("/api/v1/config").await;
  assert_eq!(status, StatusCode::NOT_FOUND);
  assert_eq!(body["error"], "feature_off");
  assert!(body["detail"].as_str().unwrap().contains("`state`"));

  let (status, body) = client.get("/api/v1/nothing-here").await;
  assert_eq!(status, StatusCode::NOT_FOUND);
  assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn health_stays_up_while_ready_follows_the_lifecycle() {
  let client = Client::new(ApiSettings::default());
  assert_eq!(client.get("/api/v1/ready").await.0, StatusCode::OK);

  client.relay.draining.store(true, Ordering::Relaxed);
  let (status, body) = client.get("/api/v1/ready").await;
  assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
  assert_eq!(body["reason"], "draining");
  assert_eq!(client.get("/api/v1/health").await.0, StatusCode::OK);

  client.relay.draining.store(false, Ordering::Relaxed);
  client.relay.accepting.store(false, Ordering::Relaxed);
  assert_eq!(client.get("/api/v1/ready").await.1["reason"], "starting");
}

#[tokio::test]
async fn read_token_is_required_except_for_health_and_ready() {
  let client = Client::new(ApiSettings {
    read_token: Some("r3ad".into()),
    write_token: Some("wr1te".into()),
    ..Default::default()
  });
  let local = "127.0.0.1:5000";
  let (status, ..) = client.send(Method::GET, "/api/v1/config", local, &[]).await;
  assert_eq!(status, StatusCode::UNAUTHORIZED);

  let wrong = [("authorization", "Bearer nope")];
  let (status, ..) = client
    .send(Method::GET, "/api/v1/config", local, &wrong)
    .await;
  assert_eq!(status, StatusCode::FORBIDDEN);

  for token in ["Bearer r3ad", "Bearer wr1te"] {
    let auth = [("authorization", token)];
    let (status, ..) = client
      .send(Method::GET, "/api/v1/config", local, &auth)
      .await;
    assert_eq!(status, StatusCode::OK, "{token} allows reads");
  }

  let (status, ..) = client.send(Method::GET, "/api/v1/ready", local, &[]).await;
  assert_eq!(status, StatusCode::OK);

  let read = [("authorization", "Bearer r3ad")];
  let (status, ..) = client
    .send(Method::POST, "/api/v1/config", local, &read)
    .await;
  assert_eq!(
    status,
    StatusCode::FORBIDDEN,
    "a read token never allows a write"
  );
}

#[tokio::test]
async fn network_bind_checks_the_client_address() {
  let client = Client::new(ApiSettings {
    bind: Some("0.0.0.0:4480".parse().unwrap()),
    allow: Some(vec!["10.0.0.0/8".into()]),
    ..Default::default()
  });
  let (status, ..) = client
    .send(Method::GET, "/api/v1/health", "10.1.2.3:5000", &[])
    .await;
  assert_eq!(status, StatusCode::OK);
  let (status, body, _) = client
    .send(Method::GET, "/api/v1/health", "192.0.2.1:5000", &[])
    .await;
  assert_eq!(status, StatusCode::FORBIDDEN);
  assert_eq!(body["error"], "address_not_allowed");
}

#[tokio::test]
async fn cors_answers_only_listed_origins() {
  let client = Client::new(ApiSettings {
    read_token: Some("r3ad".into()),
    cors_origins: vec!["http://localhost:5173".into()],
    ..Default::default()
  });
  let local = "127.0.0.1:5000";
  let allowed = [("origin", "http://localhost:5173")];
  let (status, _, headers) = client
    .send(Method::OPTIONS, "/api/v1/config", local, &allowed)
    .await;
  assert_eq!(
    status,
    StatusCode::NO_CONTENT,
    "the preflight needs no token"
  );
  assert_eq!(
    headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
    "http://localhost:5173"
  );
  assert!(
    headers[header::ACCESS_CONTROL_ALLOW_HEADERS]
      .to_str()
      .unwrap()
      .contains("Authorization")
  );

  let other = [("origin", "http://evil.example")];
  let (status, _, headers) = client
    .send(Method::OPTIONS, "/api/v1/config", local, &other)
    .await;
  assert_eq!(status, StatusCode::FORBIDDEN);
  assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));

  let with_token = [
    ("origin", "http://localhost:5173"),
    ("authorization", "Bearer r3ad"),
  ];
  let (status, _, headers) = client
    .send(Method::GET, "/api/v1/config", local, &with_token)
    .await;
  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
    "http://localhost:5173"
  );
}

#[tokio::test]
async fn connections_are_sorted_paged_and_filtered() {
  let client = Client::new(ApiSettings::default());
  let (status, body) = client
    .get("/api/v1/connections?sort=-bytes_out&limit=1")
    .await;
  assert_eq!(status, StatusCode::OK);
  assert_eq!(body["total"], 2);
  assert_eq!(body["items"][0]["conn_id"], 9);
  assert_eq!(
    body["items"][0]["announced_namespaces"], 1,
    "lists are counted"
  );

  let (_, body) = client.get("/api/v1/connections?sort=rtt").await;
  assert_eq!(body["items"][0]["conn_id"], 9);

  let (_, body) = client.get("/api/v1/connections?prefix=live").await;
  assert_eq!(body["total"], 1);
  assert_eq!(body["items"][0]["conn_id"], 7);

  let (status, body) = client.get("/api/v1/connections?sort=colour").await;
  assert_eq!(status, StatusCode::BAD_REQUEST);
  assert_eq!(body["error"], "unknown_sort_key");
}

#[tokio::test]
async fn closed_connections_are_kept_up_to_the_history_size() {
  let client = Client::new(ApiSettings {
    closed_connection_history: 2,
    ..Default::default()
  });
  assert!(client.probe().keeps_closed_connections());
  for id in [1, 2, 3] {
    let mut closed = connection(id, "live", 1.0, 1);
    closed.state = ConnectionState::Closed;
    closed.close = Some(CloseInfo {
      at_ms: 0,
      by: "peer".into(),
      code: Some(0),
      reason: "done".into(),
    });
    client.probe().connection_closed(closed);
  }
  let (_, body) = client.get("/api/v1/connections?closed=true").await;
  assert_eq!(body["total"], 2, "the oldest was dropped");
  assert_eq!(body["items"][0]["conn_id"], 3, "newest first");

  let (status, body) = client.get("/api/v1/connections/2").await;
  assert_eq!(status, StatusCode::OK);
  assert_eq!(body["close"]["reason"], "done");

  let (status, body) = client.get("/api/v1/connections/7").await;
  assert_eq!(status, StatusCode::OK, "open connections are found too");
  assert_eq!(body["implementation"], "other-relay/1.0");

  let (status, body) = client.get("/api/v1/connections/404").await;
  assert_eq!(status, StatusCode::NOT_FOUND);
  assert_eq!(body["error"], "unknown_connection");
}
