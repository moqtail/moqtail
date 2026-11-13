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

//! The endpoint index, features that are off, liveness and readiness.

mod common;

use common::*;

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
async fn meta_serves_the_configured_values_without_a_token() {
  let client = Client::new(ApiSettings::default());
  let (status, body) = client.get("/api/v1/meta").await;
  assert_eq!(
    (status, body),
    (StatusCode::OK, json!({})),
    "empty by default"
  );

  let meta = json!({ "fqdn": "relay.example.com", "region": "eu-west", "ports": [4433] });
  let client = Client::new(ApiSettings {
    read_token: Some("r3ad".into()),
    meta: meta.as_object().unwrap().clone(),
    ..Default::default()
  });
  // No token, though reads need one: apps read this before they know any token.
  let (status, body) = client.get("/api/v1/meta").await;
  assert_eq!((status, body), (StatusCode::OK, meta));
  assert_eq!(
    client.get("/api/v1/config").await.0,
    StatusCode::UNAUTHORIZED,
    "the rest of the API still needs the token"
  );
}
