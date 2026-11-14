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

//! The `actions` feature: changes to the relay, behind the write token.

mod common;

use common::*;

#[tokio::test]
async fn actions_are_off_unless_enabled() {
  let client = Client::new(ApiSettings {
    write_token: Some("wr1te".into()),
    ..Default::default()
  });
  let (status, body, _) = client
    .send(
      Method::DELETE,
      "/api/v1/connections/7",
      "127.0.0.1:5000",
      &[("authorization", "Bearer wr1te")],
    )
    .await;
  assert_eq!(
    (status, body["error"].as_str()),
    (StatusCode::NOT_FOUND, Some("feature_off"))
  );
  // The read on the same path still answers.
  assert_eq!(client.get("/api/v1/connections/7").await.0, StatusCode::OK);
  assert!(client.relay.actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn actions_need_the_write_token_and_reach_the_relay() {
  let client = Client::new(ApiSettings {
    features: Some(vec!["state".into(), "actions".into()]),
    read_token: Some("r3ad".into()),
    write_token: Some("wr1te".into()),
    ..Default::default()
  });
  let local = "127.0.0.1:5000";
  let read = [("authorization", "Bearer r3ad")];
  let write = [("authorization", "Bearer wr1te")];

  let (status, ..) = client
    .send(Method::POST, "/api/v1/connections/7/goaway", local, &read)
    .await;
  assert_eq!(status, StatusCode::FORBIDDEN);

  let (status, ..) = client
    .send(
      Method::POST,
      "/api/v1/connections/7/goaway?timeout_ms=500",
      local,
      &write,
    )
    .await;
  assert_eq!(status, StatusCode::ACCEPTED);
  let (status, ..) = client
    .send(
      Method::POST,
      "/api/v1/connections/7/goaway?timeout_ms=9999999",
      local,
      &write,
    )
    .await;
  assert_eq!(status, StatusCode::BAD_REQUEST);
  let (status, ..) = client
    .send(Method::DELETE, "/api/v1/connections/8", local, &write)
    .await;
  assert_eq!(status, StatusCode::NOT_FOUND);
  let (status, ..) = client
    .send(Method::DELETE, "/api/v1/connections/7", local, &write)
    .await;
  assert_eq!(status, StatusCode::NO_CONTENT);

  let (status, body, _) = client
    .send(Method::POST, "/api/v1/drain", local, &write)
    .await;
  assert_eq!((status, body["told"].as_u64()), (StatusCode::OK, Some(1)));

  let uri = json!({ "uri": "moqt://other.example:4433" });
  let (status, ..) = client
    .send_json(
      Method::PUT,
      "/api/v1/config/redirect_uri",
      local,
      &write,
      Some(uri),
    )
    .await;
  assert_eq!(status, StatusCode::OK);
  let (_, body, _) = client
    .send(Method::GET, "/api/v1/config/redirect_uri", local, &read)
    .await;
  assert_eq!(body["uri"], "moqt://other.example:4433");

  let (status, body, _) = client
    .send_json(
      Method::PUT,
      "/api/v1/logging",
      local,
      &write,
      Some(json!({ "filter": "!" })),
    )
    .await;
  assert_eq!(
    (status, body["error"].as_str()),
    (StatusCode::BAD_REQUEST, Some("bad_filter"))
  );
  let (status, ..) = client
    .send_json(
      Method::PUT,
      "/api/v1/logging",
      local,
      &write,
      Some(json!({ "nope": 1 })),
    )
    .await;
  assert_eq!(status, StatusCode::BAD_REQUEST);
  let (status, ..) = client
    .send_json(
      Method::PUT,
      "/api/v1/logging",
      local,
      &write,
      Some(json!({ "filter": "debug" })),
    )
    .await;
  assert_eq!(status, StatusCode::OK);

  assert_eq!(
    *client.relay.actions.lock().unwrap(),
    [
      "goaway 7 500",
      "close 8",
      "close 7",
      "drain 10000",
      "log debug"
    ]
  );
}
