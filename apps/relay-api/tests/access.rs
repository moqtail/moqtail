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

//! Tokens, the client address allowlist and cross-origin rules.

mod common;

use common::*;

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
