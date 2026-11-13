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

//! The `metrics` feature: the Prometheus output and its token.

mod common;

use common::*;

#[tokio::test]
async fn the_metrics_token_reads_the_metrics_and_nothing_else() {
  let client = Client::new(ApiSettings {
    read_token: Some("r3ad".into()),
    metrics_token: Some("m3trics".into()),
    ..Default::default()
  });
  let local = "127.0.0.1:5000";
  let metrics = [("authorization", "Bearer m3trics")];
  let (status, ..) = client
    .send(Method::GET, "/api/v1/metrics", local, &metrics)
    .await;
  assert_eq!(status, StatusCode::OK);
  let (status, ..) = client
    .send(Method::GET, "/api/v1/config", local, &metrics)
    .await;
  assert_eq!(status, StatusCode::FORBIDDEN);
  let read = [("authorization", "Bearer r3ad")];
  let (status, ..) = client
    .send(Method::GET, "/api/v1/metrics", local, &read)
    .await;
  assert_eq!(
    status,
    StatusCode::OK,
    "the read token reads the metrics too"
  );
}

#[tokio::test]
async fn a_metrics_token_alone_protects_only_the_metrics() {
  let client = Client::new(ApiSettings {
    metrics_token: Some("m3trics".into()),
    ..Default::default()
  });
  let local = "127.0.0.1:5000";
  let (status, ..) = client
    .send(Method::GET, "/api/v1/metrics", local, &[])
    .await;
  assert_eq!(status, StatusCode::UNAUTHORIZED);
  let (status, ..) = client.send(Method::GET, "/api/v1/config", local, &[]).await;
  assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn cluster_series_appear_only_on_a_clustered_relay() {
  let client = Client::new(ApiSettings::default());
  let (_, body) = client.get("/api/v1/metrics").await;
  assert!(!body.as_str().unwrap().contains("moqtail_cluster"));

  let link = |url: &str, up: bool| relay_hooks::PeerLinkInfo {
    url: url.into(),
    up,
    remote: None,
    conn_id: None,
    since_ms: None,
    last_error: None,
  };
  *client.relay.cluster.lock().unwrap() = Some(relay_hooks::ClusterInfo {
    hop_id: Some(1),
    configured_peers: vec!["moqt://b:4433".into(), "moqt://c:4433".into()],
    links: vec![link("moqt://b:4433", true), link("moqt://c:4433", false)],
    peers: vec![relay_hooks::ClusterPeerInfo {
      conn_id: 9,
      remote: "10.0.0.2:4433".into(),
      hop_id: Some(2),
      relay_cost: 1,
      adverts: vec![],
    }],
  });
  let (_, body) = client.get("/api/v1/metrics").await;
  let text = body.as_str().unwrap();
  assert!(text.contains(r#"moqtail_cluster_peer_up{peer="moqt://b:4433"} 1"#));
  assert!(text.contains(r#"moqtail_cluster_peer_up{peer="moqt://c:4433"} 0"#));
  assert!(text.contains("moqtail_cluster_peers 1"));
  assert!(text.contains(r#"moqtail_cluster_adverts{peer_hop_id="2"} 0"#));
}
