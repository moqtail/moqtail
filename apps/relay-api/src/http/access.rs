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

//! Access checks every request passes before a handler runs: the client address,
//! the token, and the browser's cross-origin rules.

use crate::ApiState;
use crate::http::error::ApiError;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::SocketAddr;
use std::sync::Arc;

/// Paths a load balancer probes without credentials. The address check still applies.
const TOKEN_EXEMPT: [&str; 2] = ["/api/v1/health", "/api/v1/ready"];
const METRICS_PATH: &str = "/api/v1/metrics";
pub(crate) const MCP_PATH: &str = "/api/v1/mcp";

/// The checks of the main listener.
pub async fn check(
  State(state): State<Arc<ApiState>>,
  ConnectInfo(peer): ConnectInfo<SocketAddr>,
  request: Request,
  next: Next,
) -> Response {
  let allow = state.config.allow.clone();
  check_with(&state, allow.as_deref(), peer, request, next).await
}

/// The checks of the metrics listener, which serves `/api/v1/metrics` only.
pub async fn check_metrics_listener(
  State(state): State<Arc<ApiState>>,
  ConnectInfo(peer): ConnectInfo<SocketAddr>,
  request: Request,
  next: Next,
) -> Response {
  let allow = state.config.metrics_allow.clone();
  check_with(&state, allow.as_deref(), peer, request, next).await
}

async fn check_with(
  state: &ApiState,
  allow: Option<&[crate::config::Cidr]>,
  peer: SocketAddr,
  request: Request,
  next: Next,
) -> Response {
  let config = &state.config;

  if let Some(allow) = allow
    && !allow.iter().any(|range| range.contains(peer.ip()))
  {
    return ApiError::new(StatusCode::FORBIDDEN, "address_not_allowed")
      .detail(format!("{} is not in api_allow", peer.ip()))
      .into_response();
  }

  let origin = allowed_origin(request.headers(), &config.cors_origins).cloned();

  // A web page could reach a local MCP server through DNS rebinding, so MCP
  // refuses any browser origin not listed. Command-line clients send no Origin.
  if request.uri().path() == MCP_PATH
    && request.headers().contains_key(header::ORIGIN)
    && origin.is_none()
  {
    return ApiError::new(StatusCode::FORBIDDEN, "origin_not_allowed")
      .detail("this Origin is not in api_cors_origins")
      .into_response();
  }

  // A browser asks before sending a request with an Authorization header. The
  // question itself carries no token, so it is answered before the token check.
  if request.method() == Method::OPTIONS {
    let mut response = StatusCode::NO_CONTENT.into_response();
    match &origin {
      Some(origin) => add_cors_headers(response.headers_mut(), origin),
      None => *response.status_mut() = StatusCode::FORBIDDEN,
    }
    return response;
  }

  if !TOKEN_EXEMPT.contains(&request.uri().path())
    && let Err(error) = check_token(
      request.method(),
      request.uri().path(),
      request.headers(),
      config,
    )
  {
    let mut response = error.into_response();
    if let Some(origin) = &origin {
      add_cors_headers(response.headers_mut(), origin);
    }
    return response;
  }

  let mut response = next.run(request).await;
  if let Some(origin) = &origin {
    add_cors_headers(response.headers_mut(), origin);
  }
  response
}

/// Reads are open unless a read token is set; the write token also allows reads,
/// and the metrics token allows reading the metrics. Anything other than a read
/// needs the write token.
fn check_token(
  method: &Method,
  path: &str,
  headers: &HeaderMap,
  config: &crate::config::ApiConfig,
) -> Result<(), ApiError> {
  let presented = headers
    .get(header::AUTHORIZATION)
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.strip_prefix("Bearer "));
  let matches = |token: &Option<String>| match (token, presented) {
    (Some(expected), Some(given)) => constant_time_eq(expected.as_bytes(), given.as_bytes()),
    _ => false,
  };

  // MCP takes its own token and nothing else; no other path accepts that token.
  if path == MCP_PATH {
    return match (&config.mcp_token, presented) {
      (Some(hash), Some(given)) if hash.verify(given) => Ok(()),
      (_, None) => Err(
        ApiError::new(StatusCode::UNAUTHORIZED, "token_required").detail(
          "send the MCP token as `Authorization: Bearer <token>`; the relay issues \
           its own tokens with `relay api-token` and does not use OAuth",
        ),
      ),
      _ => Err(ApiError::new(StatusCode::FORBIDDEN, "token_rejected")),
    };
  }

  let is_read = matches!(*method, Method::GET | Method::HEAD);
  let allowed = if is_read {
    // With only a metrics token set, the metrics need it too; other reads stay open.
    let open =
      config.read_token.is_none() && (path != METRICS_PATH || config.metrics_token.is_none());
    open
      || matches(&config.read_token)
      || matches(&config.write_token)
      || (path == METRICS_PATH && matches(&config.metrics_token))
  } else {
    matches(&config.write_token)
  };
  if allowed {
    Ok(())
  } else if presented.is_none() {
    Err(ApiError::new(StatusCode::UNAUTHORIZED, "token_required"))
  } else {
    Err(ApiError::new(StatusCode::FORBIDDEN, "token_rejected"))
  }
}

/// Compares in time that depends only on the lengths, so a caller cannot find a
/// token byte by byte from response times.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
  a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn allowed_origin<'a>(headers: &'a HeaderMap, origins: &[String]) -> Option<&'a HeaderValue> {
  let origin = headers.get(header::ORIGIN)?;
  let text = origin.to_str().ok()?;
  origins.iter().any(|o| o == text).then_some(origin)
}

fn add_cors_headers(headers: &mut HeaderMap, origin: &HeaderValue) {
  headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
  headers.insert(header::VARY, HeaderValue::from_static("Origin"));
  headers.insert(
    header::ACCESS_CONTROL_ALLOW_HEADERS,
    HeaderValue::from_static("Authorization, Content-Type"),
  );
  headers.insert(
    header::ACCESS_CONTROL_ALLOW_METHODS,
    HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
  );
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn constant_time_eq_compares_whole_values() {
    assert!(constant_time_eq(b"secret", b"secret"));
    assert!(!constant_time_eq(b"secret", b"secreT"));
    assert!(!constant_time_eq(b"secret", b"secret2"));
  }
}
