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

//! What every request-level test shares: a stand-in relay and a client that
//! sends requests through the API's full router, access checks included, as a
//! client at a given address would. Each test file uses part of it.

#![allow(dead_code, unused_imports)]

pub use axum::body::Body;
pub use axum::extract::ConnectInfo;
pub use axum::http::{Method, Request, StatusCode, header};
pub use http_body_util::BodyExt;
pub use relay_api::config::{ApiConfig, ApiSettings, RelaySettings};
pub use relay_hooks::{
  Actions, BoxFuture, CloseInfo, ConnectionInfo, ConnectionState, Inspect,
  NamespaceSubscriptionInfo, PublishedTrackInfo, QuicStats, RelayStatus, Reporter, Role,
  TrackDetail, TrackInfo,
};
pub use serde_json::{Value, json};
pub use std::net::SocketAddr;
pub use std::sync::atomic::{AtomicBool, Ordering};
pub use std::sync::{Arc, Mutex};
pub use tower::ServiceExt;

/// A relay stand-in with settable lifecycle flags.
#[derive(Default)]
pub struct FakeRelay {
  pub accepting: AtomicBool,
  pub draining: AtomicBool,
  /// What the actions asked for, in order, such as `goaway 7 10000`.
  pub actions: Mutex<Vec<String>>,
  pub redirect_uri: Mutex<Option<String>>,
  /// What `cluster` reports; a standalone relay when `None`.
  pub cluster: Mutex<Option<relay_hooks::ClusterInfo>>,
}

impl FakeRelay {
  fn record(&self, action: String) {
    self.actions.lock().unwrap().push(action);
  }
}

/// Connection 7 is the only open one.
impl Actions for FakeRelay {
  fn goaway(&self, conn_id: u64, timeout_ms: u64) -> BoxFuture<'_, bool> {
    self.record(format!("goaway {conn_id} {timeout_ms}"));
    Box::pin(async move { conn_id == 7 })
  }

  fn close(&self, conn_id: u64) -> BoxFuture<'_, bool> {
    self.record(format!("close {conn_id}"));
    Box::pin(async move { conn_id == 7 })
  }

  fn drain(&self, timeout_ms: u64) -> BoxFuture<'_, usize> {
    self.record(format!("drain {timeout_ms}"));
    self.draining.store(true, Ordering::Relaxed);
    Box::pin(async { 1 })
  }

  fn redirect_uri(&self) -> BoxFuture<'_, Option<String>> {
    Box::pin(async { self.redirect_uri.lock().unwrap().clone() })
  }

  fn set_redirect_uri(&self, uri: Option<String>) -> BoxFuture<'_, ()> {
    *self.redirect_uri.lock().unwrap() = uri;
    Box::pin(async {})
  }

  fn log_filter(&self) -> Option<String> {
    Some("info".into())
  }

  fn set_log_filter(&self, filter: &str) -> Result<(), String> {
    if filter.contains('!') {
      return Err("invalid filter directive".into());
    }
    self.record(format!("log {filter}"));
    Ok(())
  }
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
    json!({ "cache_size": 1000, "bind": "0.0.0.0:4433" })
  }

  fn connections(&self) -> BoxFuture<'_, Vec<ConnectionInfo>> {
    Box::pin(async {
      let mut publisher = connection(7, "live/room1", 10.0, 500);
      publisher.published_tracks.push(PublishedTrackInfo {
        request_id: 0,
        track: "live/room1 : video".into(),
      });
      vec![publisher, connection(9, "vod/a", 2.0, 9000)]
    })
  }

  fn tracks(&self) -> BoxFuture<'_, Vec<TrackInfo>> {
    Box::pin(async { vec![track(1, "live/room1", 3), track(2, "vod/a", 40)] })
  }

  fn track(&self, track_id: u64) -> BoxFuture<'_, Option<TrackDetail>> {
    Box::pin(async move {
      (track_id == 1).then(|| TrackDetail {
        track: track(1, "live/room1", 3),
        subscriptions: vec![],
        cached_group_ids: vec![4, 5],
        cached_bytes: 1200,
      })
    })
  }

  fn cluster(&self) -> BoxFuture<'_, relay_hooks::ClusterInfo> {
    Box::pin(async {
      self
        .cluster
        .lock()
        .unwrap()
        .clone()
        .unwrap_or(relay_hooks::ClusterInfo {
          hop_id: None,
          configured_peers: vec![],
          links: vec![],
          peers: vec![],
        })
    })
  }

  fn namespace_subscriptions(&self) -> BoxFuture<'_, Vec<NamespaceSubscriptionInfo>> {
    Box::pin(async {
      vec![NamespaceSubscriptionInfo {
        prefix: "live".into(),
        conn_id: 9,
        kind: "namespace".into(),
      }]
    })
  }
}

pub fn track(track_id: u64, namespace: &str, subscribers: usize) -> TrackInfo {
  TrackInfo {
    track_id,
    namespace: namespace.into(),
    name: "video".into(),
    origin: "publish".into(),
    status: "confirmed".into(),
    publishers: vec![],
    subscribers,
    forwarding: true,
    largest_location: Some("12:3".into()),
    cached_groups: 2,
    upstream_forward: true,
  }
}

/// A connection that announced `namespace`, with the given RTT and bytes sent.
pub fn connection(conn_id: u64, namespace: &str, rtt_ms: f64, sent: u64) -> ConnectionInfo {
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

pub struct Client {
  /// What the relay core would report to.
  reporter: Arc<dyn Reporter>,
  /// Built once, as the API does at startup, so state kept by a route (such as an
  /// MCP session) lasts across requests.
  router: axum::Router,
  pub relay: Arc<FakeRelay>,
}

impl Client {
  pub fn new(settings: ApiSettings) -> Self {
    let (config, _) = ApiConfig::from_settings(settings, RelaySettings::default()).unwrap();
    let relay = Arc::new(FakeRelay::default());
    relay.accepting.store(true, Ordering::Relaxed);
    let (router, reporter) = relay_api::router(config, relay.clone(), relay.clone());
    Client {
      reporter,
      router,
      relay,
    }
  }

  /// Sends a request from `from`, with optional extra headers, and returns the
  /// status and the parsed JSON body (`Null` when the body is empty, a string when
  /// it is not JSON).
  pub async fn send(
    &self,
    method: Method,
    path: &str,
    from: &str,
    headers: &[(&str, &str)],
  ) -> (StatusCode, Value, axum::http::HeaderMap) {
    self.send_json(method, path, from, headers, None).await
  }

  /// [`Client::send`] with a JSON body.
  pub async fn send_json(
    &self,
    method: Method,
    path: &str,
    from: &str,
    headers: &[(&str, &str)],
    json: Option<Value>,
  ) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
      request = request.header(*name, *value);
    }
    let body = match json {
      Some(json) => {
        request = request.header(header::CONTENT_TYPE, "application/json");
        Body::from(json.to_string())
      }
      None => Body::empty(),
    };
    let mut request = request.body(body).unwrap();
    let peer: SocketAddr = from.parse().unwrap();
    request.extensions_mut().insert(ConnectInfo(peer));
    let response = self.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    // Plain-text bodies, such as the metrics, come back as a JSON string.
    let body = if bytes.is_empty() {
      Value::Null
    } else {
      serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, body, headers)
  }

  pub fn reporter(&self) -> &dyn Reporter {
    self.reporter.as_ref()
  }

  pub async fn get(&self, path: &str) -> (StatusCode, Value) {
    let (status, body, _) = self.send(Method::GET, path, "127.0.0.1:5000", &[]).await;
    (status, body)
  }
}
