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

//! The endpoint table and the handlers for the index, health, readiness and
//! configuration.

use crate::ApiState;
use crate::config::Feature;
use crate::error::ApiError;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get};
use axum::{Json, Router, middleware};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

/// One row of the endpoint index.
struct Endpoint {
  method: &'static str,
  path: &'static str,
  feature: Feature,
  route: fn() -> MethodRouter<Arc<ApiState>>,
}

/// Every endpoint the API serves. The index is built from this table, so it cannot
/// list an endpoint that does not exist.
const ENDPOINTS: &[Endpoint] = &[
  Endpoint {
    method: "GET",
    path: "/api/v1",
    feature: Feature::Health,
    route: || get(index),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/health",
    feature: Feature::Health,
    route: || get(health),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/ready",
    feature: Feature::Health,
    route: || get(ready),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/config",
    feature: Feature::State,
    route: || get(config),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/connections",
    feature: Feature::State,
    route: || get(crate::connections::list),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/connections/{conn_id}",
    feature: Feature::State,
    route: || get(crate::connections::detail),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/qlog",
    feature: Feature::Qlog,
    route: || get(crate::qlog::index),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/qlog/{conn_id}",
    feature: Feature::Qlog,
    route: || get(crate::qlog::trace),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/qlog/{conn_id}/quic",
    feature: Feature::Qlog,
    route: || get(crate::qlog::quic_trace),
  },
];

pub fn router(state: Arc<ApiState>) -> Router {
  let mut router = Router::new();
  for endpoint in ENDPOINTS {
    let route = if state.config.has(endpoint.feature) {
      (endpoint.route)()
    } else {
      let feature = endpoint.feature;
      axum::routing::any(move || async move { feature_off(feature) })
    };
    router = router.route(endpoint.path, route);
  }
  router
    .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "not_found").into_response() })
    .layer(middleware::from_fn_with_state(
      state.clone(),
      crate::access::check,
    ))
    .with_state(state)
}

fn feature_off(feature: Feature) -> Response {
  ApiError::new(StatusCode::NOT_FOUND, "feature_off")
    .detail(format!(
      "add `{}` to api_features to turn this endpoint on",
      feature.name()
    ))
    .into_response()
}

#[derive(Serialize)]
struct IndexEntry {
  method: &'static str,
  path: &'static str,
  feature: &'static str,
  enabled: bool,
}

// Not `async fn`: the endpoint table refers to this handler and this handler reads
// the table, and a named future type breaks that cycle for the compiler.
fn index(State(state): State<Arc<ApiState>>) -> std::future::Ready<Json<serde_json::Value>> {
  let endpoints: Vec<IndexEntry> = ENDPOINTS
    .iter()
    .map(|e| IndexEntry {
      method: e.method,
      path: e.path,
      feature: e.feature.name(),
      enabled: state.config.has(e.feature),
    })
    .collect();
  let features: Vec<&str> = state.config.features.iter().map(|f| f.name()).collect();
  std::future::ready(Json(json!({
    "version": state.inspect.status().version,
    "features": features,
    "endpoints": endpoints,
  })))
}

/// Liveness: the process is up and answering, including while it drains.
async fn health() -> Json<serde_json::Value> {
  Json(json!({ "status": "ok" }))
}

/// Readiness: whether the relay takes new sessions. Load balancers stop sending
/// clients while this is 503.
async fn ready(State(state): State<Arc<ApiState>>) -> Response {
  let status = state.inspect.status();
  let reason = if status.draining {
    Some("draining")
  } else if !status.accepting {
    Some("starting")
  } else {
    None
  };
  match reason {
    None => Json(json!({ "ready": true })).into_response(),
    Some(reason) => (
      StatusCode::SERVICE_UNAVAILABLE,
      Json(json!({ "ready": false, "reason": reason })),
    )
      .into_response(),
  }
}

async fn config(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
  Json(state.inspect.config())
}
