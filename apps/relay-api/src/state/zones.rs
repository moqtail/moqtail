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

//! Accounting zones: traffic counted per namespace prefix listed in
//! `accounting_zones`, for per-tenant numbers on a CDN. A namespace belongs to
//! the longest prefix it falls under, or to no zone.

use crate::ApiState;
use axum::Json;
use axum::extract::State;
use relay_hooks::stats::ZoneCounters;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// What is counted for one zone besides its traffic.
#[derive(Debug, Default)]
struct ZoneRequests {
  ok: AtomicU64,
  error: AtomicU64,
}

#[derive(Debug)]
pub(crate) struct Zones {
  /// The configured prefixes, as `a/b`.
  names: Vec<String>,
  traffic: Vec<Arc<ZoneCounters>>,
  requests: Vec<ZoneRequests>,
  /// Everything outside the listed prefixes; the last slot of each list.
  unzoned: usize,
}

impl Zones {
  pub fn new(prefixes: &[String]) -> Self {
    let names: Vec<String> = prefixes
      .iter()
      .map(|p| p.trim_matches('/').to_string())
      .collect();
    let slots = names.len() + 1;
    Zones {
      unzoned: names.len(),
      names,
      traffic: (0..slots).map(|_| Arc::default()).collect(),
      requests: (0..slots).map(|_| ZoneRequests::default()).collect(),
    }
  }

  pub fn is_empty(&self) -> bool {
    self.names.is_empty()
  }

  /// The slot of the longest prefix `namespace` (`a/b/c`) falls under.
  fn slot(&self, namespace: &str) -> usize {
    self
      .names
      .iter()
      .enumerate()
      .filter(|(_, prefix)| {
        namespace == prefix.as_str()
          || namespace
            .strip_prefix(prefix.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
      })
      .max_by_key(|(_, prefix)| prefix.len())
      .map_or(self.unzoned, |(i, _)| i)
  }

  pub fn traffic_for(&self, namespace: &str) -> Arc<ZoneCounters> {
    self.traffic[self.slot(namespace)].clone()
  }

  /// Counts an answered request about `namespace`.
  pub fn request_answered(&self, namespace: &str, ok: bool) {
    let requests = &self.requests[self.slot(namespace)];
    if ok {
      requests.ok.fetch_add(1, Ordering::Relaxed);
    } else {
      requests.error.fetch_add(1, Ordering::Relaxed);
    }
  }

  fn name(&self, slot: usize) -> &str {
    self.names.get(slot).map_or("unzoned", String::as_str)
  }
}

#[derive(Debug, Serialize)]
pub struct ZoneReport {
  zone: String,
  tracks: usize,
  publishers: usize,
  subscribers: usize,
  objects_in: u64,
  bytes_in: u64,
  objects_out: u64,
  bytes_out: u64,
  requests_ok: u64,
  requests_error: u64,
}

/// Every zone, then `unzoned` for the rest. Empty when no zones are configured.
impl ZoneReport {
  pub fn zone(&self) -> &str {
    &self.zone
  }

  pub fn objects(&self) -> (u64, u64) {
    (self.objects_in, self.objects_out)
  }

  pub fn bytes(&self) -> (u64, u64) {
    (self.bytes_in, self.bytes_out)
  }

  pub fn requests(&self) -> (u64, u64) {
    (self.requests_ok, self.requests_error)
  }
}

pub fn reports(zones: &Zones, tracks: &[relay_hooks::TrackInfo]) -> Vec<ZoneReport> {
  if zones.is_empty() {
    return Vec::new();
  }
  (0..=zones.unzoned)
    .map(|slot| {
      let in_zone: Vec<_> = tracks
        .iter()
        .filter(|t| zones.slot(&t.namespace) == slot)
        .collect();
      let traffic = &zones.traffic[slot];
      let requests = &zones.requests[slot];
      ZoneReport {
        zone: zones.name(slot).to_string(),
        tracks: in_zone.len(),
        publishers: in_zone.iter().map(|t| t.publishers.len()).sum(),
        subscribers: in_zone.iter().map(|t| t.subscribers).sum(),
        objects_in: traffic.objects_in.get(),
        bytes_in: traffic.bytes_in.get(),
        objects_out: traffic.objects_out.get(),
        bytes_out: traffic.bytes_out.get(),
        requests_ok: requests.ok.load(Ordering::Relaxed),
        requests_error: requests.error.load(Ordering::Relaxed),
      }
    })
    .collect()
}

pub async fn list(State(state): State<Arc<ApiState>>) -> Json<Vec<ZoneReport>> {
  let tracks = state.tracks().await;
  Json(reports(&state.zones, &tracks))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_longest_prefix_wins_and_matches_whole_fields() {
    let zones = Zones::new(&["live".into(), "live/sports/".into(), "vod".into()]);
    assert_eq!(zones.name(zones.slot("live/news")), "live");
    assert_eq!(zones.name(zones.slot("live/sports/match1")), "live/sports");
    assert_eq!(zones.name(zones.slot("live")), "live");
    assert_eq!(zones.name(zones.slot("livestream")), "unzoned");
    assert_eq!(zones.name(zones.slot("other")), "unzoned");
  }

  #[test]
  fn traffic_and_requests_are_counted_per_zone() {
    let zones = Zones::new(&["live".into()]);
    zones.traffic_for("live/a").object_in(100);
    zones.traffic_for("x").object_out(10);
    zones.request_answered("live/a", false);
    let reports = reports(&zones, &[]);
    assert_eq!((reports[0].bytes_in, reports[0].requests_error), (100, 1));
    assert_eq!(
      (reports[1].zone.as_str(), reports[1].bytes_out),
      ("unzoned", 10)
    );
  }
}
