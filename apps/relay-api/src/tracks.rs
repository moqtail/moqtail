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

//! `/api/v1/tracks`: the tracks the relay knows, and how they fan out.

use crate::ApiState;
use crate::error::ApiError;
use crate::list::{ListQuery, Page, SortValue, page, parse_query};
use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use moqtail_probe::{TrackDetail, TrackInfo};
use std::sync::Arc;

const SORT_KEYS: &[&str] = &["track_id", "subscribers", "publishers", "cached_groups"];

pub async fn list(
  State(state): State<Arc<ApiState>>,
  query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<Page<TrackInfo>>, ApiError> {
  let query = parse_query(query)?;
  let tracks = state.tracks().await;
  let matching: Vec<TrackInfo> = tracks
    .iter()
    .filter(|t| {
      query
        .prefix
        .as_ref()
        .is_none_or(|p| t.namespace.starts_with(p.as_str()))
    })
    .cloned()
    .collect();
  let page = page(matching, &query, SORT_KEYS, |t, key| {
    Some(SortValue::Number(match key {
      "track_id" => t.track_id as f64,
      "subscribers" => t.subscribers as f64,
      "publishers" => t.publishers.len() as f64,
      "cached_groups" => t.cached_groups as f64,
      _ => return None,
    }))
  })?;
  Ok(Json(page))
}

/// One track with every subscription and its cache contents. Not cached: it is
/// read for one track at a time.
pub async fn detail(
  State(state): State<Arc<ApiState>>,
  Path(track_id): Path<u64>,
) -> Result<Json<TrackDetail>, ApiError> {
  state
    .inspect
    .track(track_id)
    .await
    .map(Json)
    .ok_or_else(|| {
      ApiError::new(StatusCode::NOT_FOUND, "unknown_track").detail(format!("no track {track_id}"))
    })
}
