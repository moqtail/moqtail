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

//! `/api/v1/overview`, `/api/v1/namespaces` and `/api/v1/upstreams`: views
//! assembled from the connection and track snapshots.

use crate::ApiState;
use crate::control::{ResultCounts, UpstreamCounts};
use axum::Json;
use axum::extract::State;
use moqtail_probe::{ClusterInfo, ConnectionState, NamespaceSubscriptionInfo, RelayStatus, Role};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Serialize)]
pub struct Overview {
  #[serde(flatten)]
  status: RelayStatus,
  /// The API's features that are on.
  features: Vec<&'static str>,
  /// Open connections by state.
  connections: BTreeMap<&'static str, usize>,
  closed_connections_kept: usize,
  tracks: usize,
  subscriptions: usize,
  cached_groups: u64,
  /// Request results by `type/direction`.
  requests: BTreeMap<String, ResultCounts>,
  /// Running totals of the data path, when the `metrics` feature counts them.
  /// Rates are the difference between two reads.
  #[serde(skip_serializing_if = "Option::is_none")]
  traffic: Option<Traffic>,
}

#[derive(Serialize)]
pub struct Traffic {
  objects_received: u64,
  bytes_received: u64,
  objects_sent: u64,
  bytes_sent: u64,
  objects_dropped: u64,
}

pub async fn overview(State(state): State<Arc<ApiState>>) -> Json<Overview> {
  let connections = state.open_connections().await;
  let tracks = state.tracks().await;
  let mut by_state = BTreeMap::new();
  for c in connections.iter() {
    let name = match c.state {
      ConnectionState::Active => "active",
      ConnectionState::Idle => "idle",
      ConnectionState::Draining => "draining",
      ConnectionState::Closed => "closed",
    };
    *by_state.entry(name).or_insert(0) += 1;
  }
  Json(Overview {
    status: state.inspect.status(),
    features: state.config.features.iter().map(|f| f.name()).collect(),
    connections: by_state,
    closed_connections_kept: state.probe.closed_connections().len(),
    tracks: tracks.len(),
    subscriptions: connections.iter().map(|c| c.subscriptions.len()).sum(),
    cached_groups: tracks.iter().map(|t| t.cached_groups).sum(),
    requests: state.control.results(),
    traffic: state.counters.as_deref().map(|c| Traffic {
      objects_received: c.objects_received.get(),
      bytes_received: c.bytes_received.get(),
      objects_sent: c.objects_sent.get() + c.fetch_objects_sent.get(),
      bytes_sent: c.bytes_sent.get() + c.fetch_bytes_sent.get(),
      objects_dropped: c.dropped_queue_full.get() + c.dropped_delivery_timeout.get(),
    }),
  })
}

#[derive(Serialize)]
pub struct Namespaces {
  /// Each announced namespace and the connections that announced it.
  announced: Vec<Announced>,
  subscriptions: Vec<NamespaceSubscriptionInfo>,
}

#[derive(Serialize)]
pub struct Announced {
  namespace: String,
  announced_by: Vec<u64>,
}

pub async fn namespaces(State(state): State<Arc<ApiState>>) -> Json<Namespaces> {
  let connections = state.open_connections().await;
  let mut announced: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
  for c in connections.iter() {
    for ns in &c.announced_namespaces {
      announced.entry(ns).or_default().push(c.conn_id);
    }
  }
  Json(Namespaces {
    announced: announced
      .into_iter()
      .map(|(namespace, announced_by)| Announced {
        namespace: namespace.to_string(),
        announced_by,
      })
      .collect(),
    subscriptions: state.inspect.namespace_subscriptions().await,
  })
}

#[derive(Serialize)]
pub struct Upstream {
  conn_id: u64,
  /// `publisher` or `cluster_peer`.
  kind: &'static str,
  /// True while its connection is open; a closed source is no longer listed.
  up: bool,
  remote: String,
  tracks: usize,
  announced_namespaces: usize,
  rtt_ms: f64,
  #[serde(flatten)]
  requests: UpstreamCounts,
}

/// Every open connection the relay can pull from: publishers and cluster peers.
pub async fn upstreams(State(state): State<Arc<ApiState>>) -> Json<Vec<Upstream>> {
  let connections = state.open_connections().await;
  let list = connections
    .iter()
    .filter_map(|c| {
      let counts = state.control.upstream(c.conn_id);
      let is_source = c.role == Role::ClusterPeer
        || !c.published_tracks.is_empty()
        || !c.announced_namespaces.is_empty()
        || counts.is_some();
      is_source.then(|| Upstream {
        conn_id: c.conn_id,
        kind: if c.role == Role::ClusterPeer {
          "cluster_peer"
        } else {
          "publisher"
        },
        up: true,
        remote: c.remote.clone(),
        tracks: c.published_tracks.len(),
        announced_namespaces: c.announced_namespaces.len(),
        rtt_ms: c.quic.rtt_ms,
        requests: counts.unwrap_or_default(),
      })
    })
    .collect();
  Json(list)
}

/// This relay's cluster identity, its peer links and what it advertises over them.
pub async fn cluster(State(state): State<Arc<ApiState>>) -> Json<ClusterInfo> {
  Json(state.inspect.cluster().await)
}
