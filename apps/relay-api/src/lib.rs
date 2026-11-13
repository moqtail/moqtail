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

//! The relay's HTTP API under `/api/v1`.
//!
//! It runs on its own small tokio runtime, so serving requests, encoding JSON and
//! slow clients never take time from the relay's delivery threads. It reads the
//! relay only through [`relay_hooks::Inspect`], and changes it only through
//! [`relay_hooks::Actions`].

mod access;
mod actions;
pub mod config;
mod connections;
mod control;
mod error;
mod events;
mod latency;
mod list;
mod mcp;
mod metrics;
mod metrics_track;
mod overview;
mod qlog;
mod reporter;
mod routes;
mod snapshot;
mod timeline;
pub mod token;
mod tracks;
mod zones;

use anyhow::{Context, Result};
use config::{ApiConfig, Feature};
use control::ControlTracker;
use relay_hooks::{Actions, ConnectionInfo, Inspect, Reporter, TrackInfo};
use reporter::{ApiReporter, Connection};
use snapshot::Cached;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};

/// What every handler can reach.
pub(crate) struct ApiState {
  pub config: ApiConfig,
  pub inspect: Arc<dyn Inspect>,
  /// Changes to the relay; used only by the `actions` feature.
  pub actions: Arc<dyn Actions>,
  pub reporter: Arc<ApiReporter>,
  pub control: Arc<ControlTracker>,
  pub zones: Arc<zones::Zones>,
  /// The data path's counters; `None` unless the `metrics` feature is on.
  pub counters: Option<Arc<relay_hooks::stats::RelayCounters>>,
  connections: Cached<Vec<ConnectionInfo>>,
  tracks: Cached<Vec<TrackInfo>>,
}

impl ApiState {
  pub(crate) fn new(
    config: ApiConfig,
    inspect: Arc<dyn Inspect>,
    actions: Arc<dyn Actions>,
  ) -> Self {
    let zones = Arc::new(zones::Zones::new(&config.accounting_zones));
    let control = Arc::new(ControlTracker::new(config.upstream_timeout, zones.clone()));
    let latency = (config.has(Feature::Latency) || config.metrics_track.is_some()).then(|| {
      Arc::new(latency::Latency::new(
        config.latency_detail,
        config.latency_sample,
        config.slowlog,
      ))
    });
    let counters =
      (config.has(Feature::Metrics) || config.metrics_track.is_some()).then(Arc::default);
    let reporter = Arc::new(ApiReporter::new(
      config.has(Feature::State) || config.has(Feature::Metrics),
      config.closed_connection_history,
      control.clone(),
      latency,
      zones.clone(),
    ));
    ApiState {
      config,
      inspect,
      actions,
      reporter,
      control,
      zones,
      counters,
      connections: Cached::new(),
      tracks: Cached::new(),
    }
  }

  /// The open connections, at most a second old.
  pub async fn open_connections(&self) -> Arc<Vec<ConnectionInfo>> {
    self.connections.get(|| self.inspect.connections()).await
  }

  /// The open connections with their control message and request counts.
  pub async fn open_connections_with_counts(&self) -> Vec<Connection> {
    self
      .open_connections()
      .await
      .iter()
      .map(|info| Connection {
        counts: self.control.connection(info.conn_id),
        info: info.clone(),
      })
      .collect()
  }

  /// A track's processing time over the last minute, when it is measured.
  pub fn track_processing(&self, track_id: u64) -> Option<serde_json::Value> {
    let summary = self.reporter.latency.as_ref()?.track_summary(track_id)?;
    serde_json::to_value(summary).ok()
  }

  /// Adds `processing_ms` to the JSON of subscription `request_id` of connection
  /// `conn_id`, when it is measured.
  pub fn add_subscription_processing(&self, conn_id: u64, subscription: &mut serde_json::Value) {
    let Some(latency) = &self.reporter.latency else {
      return;
    };
    if let Some(request_id) = subscription["request_id"].as_u64()
      && let Some(summary) = latency.subscription_summary(conn_id, request_id)
      && let Ok(value) = serde_json::to_value(summary)
    {
      subscription["processing_ms"] = value;
    }
  }

  /// Every track, at most a second old.
  pub async fn tracks(&self) -> Arc<Vec<TrackInfo>> {
    self.tracks.get(|| self.inspect.tracks()).await
  }
}

/// Binds the API's listener and serves it from a dedicated thread. Returns once the
/// socket is bound, so a port already in use is reported at relay startup. The
/// returned reporter is the one the relay core should report to.
pub fn start(
  config: ApiConfig,
  inspect: Arc<dyn Inspect>,
  actions: Arc<dyn Actions>,
) -> Result<Arc<dyn Reporter>> {
  let main = bind_listener(config.bind)?;
  let metrics = config.metrics_bind.map(bind_listener).transpose()?;
  let (bind, metrics_bind) = (config.bind, config.metrics_bind);
  let state = Arc::new(ApiState::new(config, inspect, actions));
  let reporter: Arc<dyn Reporter> = state.reporter.clone();
  if state.config.has(Feature::State)
    || state.config.has(Feature::Metrics)
    || state.config.metrics_track.is_some()
  {
    moqtail::transport::observe::set_control_observer(state.control.clone());
  }
  if let Some(counters) = &state.counters {
    relay_hooks::install_counters(counters.clone());
  }
  if state.config.has(Feature::Events) {
    // Builds events for listeners when no qlog files are written; does nothing
    // when they are.
    moqtail::transport::qlog::init_live();
  }

  std::thread::Builder::new()
    .name("relay-api".into())
    .spawn(move || {
      let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("relay-api-worker")
        .enable_all()
        .build()
      {
        Ok(runtime) => runtime,
        Err(e) => {
          error!("API runtime failed to start: {e}");
          return;
        }
      };
      if let Some(latency) = state.reporter.latency.clone() {
        runtime.spawn(latency::copy_periodically(latency));
      }
      if let Some((relay, resource_id)) = state.config.metrics_track.clone() {
        runtime.spawn(metrics_track::run(state.clone(), relay, resource_id));
      }
      if let Some(listener) = metrics {
        runtime.spawn(serve(listener, routes::metrics_router(state.clone())));
      }
      runtime.block_on(serve(main, routes::router(state)));
    })?;
  info!("API listening on http://{bind}/api/v1");
  if let Some(metrics_bind) = metrics_bind {
    info!("API metrics listening on http://{metrics_bind}/api/v1/metrics");
  }
  Ok(reporter)
}

/// Binds a listener now, so a port already in use is reported at relay startup.
fn bind_listener(addr: SocketAddr) -> Result<std::net::TcpListener> {
  let listener =
    std::net::TcpListener::bind(addr).with_context(|| format!("cannot bind the API to {addr}"))?;
  listener.set_nonblocking(true)?;
  Ok(listener)
}

async fn serve(listener: std::net::TcpListener, router: axum::Router) {
  let addr = listener.local_addr().ok();
  let listener = match tokio::net::TcpListener::from_std(listener) {
    Ok(listener) => listener,
    Err(e) => {
      error!("API listener on {addr:?} failed: {e}");
      return;
    }
  };
  let app = router.into_make_service_with_connect_info::<SocketAddr>();
  if let Err(e) = axum::serve(listener, app).await {
    error!("API server on {addr:?} stopped: {e}");
  }
}

#[cfg(test)]
mod tests;
