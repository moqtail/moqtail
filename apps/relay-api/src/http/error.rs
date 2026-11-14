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

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// The body of every error response: `{"error": "...", "detail": "..."}`.
#[derive(Debug, Serialize)]
pub struct ApiError {
  #[serde(skip)]
  status: StatusCode,
  /// A stable, machine-readable code such as `feature_off`.
  error: &'static str,
  /// A sentence for a person, saying what went wrong and how to fix it.
  #[serde(skip_serializing_if = "Option::is_none")]
  detail: Option<String>,
}

impl ApiError {
  pub fn new(status: StatusCode, error: &'static str) -> Self {
    ApiError {
      status,
      error,
      detail: None,
    }
  }

  pub fn detail(mut self, detail: impl Into<String>) -> Self {
    self.detail = Some(detail.into());
    self
  }
}

impl IntoResponse for ApiError {
  fn into_response(self) -> Response {
    (self.status, Json(&self)).into_response()
  }
}
