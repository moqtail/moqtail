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

//! The `state` feature: connections, tracks, the overview, namespaces and upstreams.

mod common;

use common::*;

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
  assert!(client.reporter().keeps_closed_connections());
  for id in [1, 2, 3] {
    let mut closed = connection(id, "live", 1.0, 1);
    closed.state = ConnectionState::Closed;
    closed.close = Some(CloseInfo {
      at_ms: 0,
      by: "peer".into(),
      code: Some(0),
      reason: "done".into(),
    });
    client.reporter().connection_closed(closed);
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

#[tokio::test]
async fn tracks_list_sort_and_detail() {
  let client = Client::new(ApiSettings::default());
  let (_, body) = client.get("/api/v1/tracks?sort=-subscribers").await;
  assert_eq!(body["items"][0]["track_id"], 2);
  let (_, body) = client.get("/api/v1/tracks?prefix=live").await;
  assert_eq!(body["total"], 1);

  let (status, body) = client.get("/api/v1/tracks/1").await;
  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    body["namespace"], "live/room1",
    "the track's fields are flattened in"
  );
  assert_eq!(body["cached_group_ids"], json!([4, 5]));
  assert_eq!(
    client.get("/api/v1/tracks/3").await.0,
    StatusCode::NOT_FOUND
  );
}

#[tokio::test]
async fn overview_namespaces_and_upstreams() {
  let client = Client::new(ApiSettings::default());
  let (_, body) = client.get("/api/v1/overview").await;
  assert_eq!(body["connections"]["active"], 2);
  assert_eq!(body["tracks"], 2);
  assert_eq!(body["cached_groups"], 4);
  assert_eq!(body["version"], "moqtail/test");

  let (_, body) = client.get("/api/v1/namespaces").await;
  assert_eq!(body["announced"][0]["namespace"], "live/room1");
  assert_eq!(body["announced"][0]["announced_by"], json!([7]));
  assert_eq!(body["subscriptions"][0]["kind"], "namespace");

  let (_, body) = client.get("/api/v1/upstreams").await;
  let ids: Vec<u64> = body
    .as_array()
    .unwrap()
    .iter()
    .map(|u| u["conn_id"].as_u64().unwrap())
    .collect();
  assert_eq!(
    ids,
    vec![7, 9],
    "both announced a namespace, so both are sources"
  );
  assert_eq!(body[0]["tracks"], 1);
}
