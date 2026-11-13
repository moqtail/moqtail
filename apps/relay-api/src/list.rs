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

//! Sorting and paging of list endpoints. The relay answers with one page, so a
//! client asking for "the top 20 by bytes out" on a busy relay gets 20 entries.

use crate::error::ApiError;
use axum::extract::Query;
use axum::extract::rejection::QueryRejection;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub const DEFAULT_LIMIT: usize = 100;
pub const MAX_LIMIT: usize = 1000;

/// The query parameters every list endpoint takes.
#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
  /// A sort key; a leading `-` sorts in descending order.
  pub sort: Option<String>,
  pub limit: Option<usize>,
  pub offset: Option<usize>,
  /// Only entries that concern a namespace starting with this.
  pub prefix: Option<String>,
}

/// The query string parsed into `T`, or a JSON error saying what was wrong with it.
pub fn parse_query<T>(query: Result<Query<T>, QueryRejection>) -> Result<T, ApiError> {
  query
    .map(|Query(q)| q)
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "bad_query").detail(e.body_text()))
}

/// One page of a list, with the size of the whole list.
#[derive(Debug, Serialize)]
pub struct Page<T> {
  pub total: usize,
  pub offset: usize,
  pub limit: usize,
  pub items: Vec<T>,
}

/// A value to sort by.
#[derive(Debug, PartialEq, PartialOrd)]
pub enum SortValue {
  Number(f64),
}

/// Sorts `items` by the query's key, then returns the requested page. `key` maps a
/// sort key name to each item's value, and returns `None` for an unknown name.
pub fn page<T>(
  mut items: Vec<T>,
  query: &ListQuery,
  keys: &[&str],
  key: impl Fn(&T, &str) -> Option<SortValue>,
) -> Result<Page<T>, ApiError> {
  if let Some(sort) = &query.sort {
    let (name, descending) = match sort.strip_prefix('-') {
      Some(name) => (name, true),
      None => (sort.as_str(), false),
    };
    if !keys.contains(&name) {
      return Err(
        ApiError::new(StatusCode::BAD_REQUEST, "unknown_sort_key")
          .detail(format!("sort by one of: {}", keys.join(", "))),
      );
    }
    items.sort_by(|a, b| {
      let order = key(a, name)
        .partial_cmp(&key(b, name))
        .unwrap_or(Ordering::Equal);
      if descending { order.reverse() } else { order }
    });
  }
  let total = items.len();
  let offset = query.offset.unwrap_or(0).min(total);
  let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
  let items = items.into_iter().skip(offset).take(limit).collect();
  Ok(Page {
    total,
    offset,
    limit,
    items,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn by_value(v: &i32, _: &str) -> Option<SortValue> {
    Some(SortValue::Number(*v as f64))
  }

  #[test]
  fn sorts_and_pages() {
    let query = ListQuery {
      sort: Some("-n".into()),
      limit: Some(2),
      offset: Some(1),
      ..Default::default()
    };
    let page = page(vec![3, 1, 4, 1, 5], &query, &["n"], by_value).unwrap();
    assert_eq!(page.total, 5);
    assert_eq!(page.items, vec![4, 3]);
  }

  #[test]
  fn rejects_an_unknown_key_and_clamps_the_page() {
    let unknown = ListQuery {
      sort: Some("nope".into()),
      ..Default::default()
    };
    assert!(page(vec![1], &unknown, &["n"], by_value).is_err());

    let past_the_end = ListQuery {
      offset: Some(10),
      limit: Some(5000),
      ..Default::default()
    };
    let page = page(vec![1, 2], &past_the_end, &["n"], by_value).unwrap();
    assert_eq!(
      (page.offset, page.limit, page.items.len()),
      (2, MAX_LIMIT, 0)
    );
  }
}
