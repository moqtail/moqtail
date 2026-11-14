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

pub mod access;
pub mod error;
pub mod list;
pub mod snapshot;

use crate::ApiState;
use crate::config::Feature;
use crate::http::error::ApiError;
use axum::extract::State;
use axum::http::Method;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, delete, get, on, post, put};
use axum::{Json, Router, middleware};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

/// One row of the endpoint index.
struct Endpoint {
  method: &'static str,
  path: &'static str,
  feature: Feature,
  /// Builds the route; the state is there for routes that need it at build time.
  route: fn(&Arc<ApiState>) -> MethodRouter<Arc<ApiState>>,
}

/// Every endpoint the API serves. The index is built from this table, so it cannot
/// list an endpoint that does not exist.
const ENDPOINTS: &[Endpoint] = &[
  Endpoint {
    method: "GET",
    path: "/api/v1",
    feature: Feature::Health,
    route: |_| get(index),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/health",
    feature: Feature::Health,
    route: |_| get(health),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/ready",
    feature: Feature::Health,
    route: |_| get(ready),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/meta",
    feature: Feature::Health,
    route: |_| get(meta),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/config",
    feature: Feature::State,
    route: |_| get(config),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/connections",
    feature: Feature::State,
    route: |_| get(crate::state::connections::list),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/connections/{conn_id}",
    feature: Feature::State,
    route: |_| get(crate::state::connections::detail),
  },
  Endpoint {
    method: "DELETE",
    path: "/api/v1/connections/{conn_id}",
    feature: Feature::Actions,
    route: |_| delete(crate::actions::close),
  },
  Endpoint {
    method: "POST",
    path: "/api/v1/connections/{conn_id}/goaway",
    feature: Feature::Actions,
    route: |_| post(crate::actions::goaway),
  },
  Endpoint {
    method: "POST",
    path: "/api/v1/drain",
    feature: Feature::Actions,
    route: |_| post(crate::actions::drain),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/config/redirect_uri",
    feature: Feature::Actions,
    route: |_| get(crate::actions::redirect_uri),
  },
  Endpoint {
    method: "PUT",
    path: "/api/v1/config/redirect_uri",
    feature: Feature::Actions,
    route: |_| put(crate::actions::set_redirect_uri),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/logging",
    feature: Feature::Actions,
    route: |_| get(crate::actions::logging),
  },
  Endpoint {
    method: "PUT",
    path: "/api/v1/logging",
    feature: Feature::Actions,
    route: |_| put(crate::actions::set_logging),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/overview",
    feature: Feature::State,
    route: |_| get(crate::state::overview::overview),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/tracks",
    feature: Feature::State,
    route: |_| get(crate::state::tracks::list),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/tracks/{track_id}",
    feature: Feature::State,
    route: |_| get(crate::state::tracks::detail),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/namespaces",
    feature: Feature::State,
    route: |_| get(crate::state::overview::namespaces),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/upstreams",
    feature: Feature::State,
    route: |_| get(crate::state::overview::upstreams),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/latency",
    feature: Feature::Latency,
    route: |_| get(crate::latency::report),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/latency/slow",
    feature: Feature::Latency,
    route: |_| get(crate::latency::slowlog),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/zones",
    feature: Feature::State,
    route: |_| get(crate::state::zones::list),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/cluster",
    feature: Feature::State,
    route: |_| get(crate::state::overview::cluster),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/events",
    feature: Feature::Events,
    route: |_| get(crate::qlog::events::events),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/metrics",
    feature: Feature::Metrics,
    route: |_| get(crate::metrics::metrics),
  },
  Endpoint {
    method: "POST",
    path: "/api/v1/mcp",
    feature: Feature::Mcp,
    route: |state| axum::routing::any_service(crate::mcp::service(state.clone())),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/qlog",
    feature: Feature::Qlog,
    route: |_| get(crate::qlog::index),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/qlog/{conn_id}",
    feature: Feature::Qlog,
    route: |_| get(crate::qlog::trace),
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/qlog/{conn_id}/quic",
    feature: Feature::Qlog,
    route: |_| get(crate::qlog::quic_trace),
  },
];

pub fn router(state: Arc<ApiState>) -> Router {
  // Several rows can share a path with different methods; their routes merge.
  let mut paths: BTreeMap<&str, MethodRouter<Arc<ApiState>>> = BTreeMap::new();
  for endpoint in ENDPOINTS {
    let route = if state.config.has(endpoint.feature) {
      (endpoint.route)(&state)
    } else {
      let feature = endpoint.feature;
      let method = Method::from_bytes(endpoint.method.as_bytes()).expect("a known method");
      on(
        MethodFilter::try_from(method).expect("a routable method"),
        move || async move { feature_off(feature) },
      )
    };
    let merged = match paths.remove(endpoint.path) {
      Some(existing) => existing.merge(route),
      None => route,
    };
    paths.insert(endpoint.path, merged);
  }
  let mut router = Router::new();
  for (path, route) in paths {
    router = router.route(path, route);
  }
  router
    .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "not_found").into_response() })
    .layer(middleware::from_fn_with_state(
      state.clone(),
      crate::http::access::check,
    ))
    .with_state(state)
}

/// The metrics listener's router: `/api/v1/metrics` and nothing else.
pub fn metrics_router(state: Arc<ApiState>) -> Router {
  let route = if state.config.has(Feature::Metrics) {
    get(crate::metrics::metrics)
  } else {
    axum::routing::any(|| async { feature_off(Feature::Metrics) })
  };
  Router::new()
    .route("/api/v1/metrics", route)
    .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "not_found").into_response() })
    .layer(middleware::from_fn_with_state(
      state.clone(),
      crate::http::access::check_metrics_listener,
    ))
    .with_state(state)
}

fn feature_off(feature: Feature) -> Response {
  ApiError::new(StatusCode::NOT_FOUND, "feature_off")
    .detail(format!(
      "add `{}` to api.features to turn this endpoint on",
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
/// `api.meta` from the relay's configuration, as written: whatever the operator
/// wants apps to know, such as the relay's public host name.
async fn meta(
  State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Map<String, serde_json::Value>> {
  Json(state.config.meta.clone())
}

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
