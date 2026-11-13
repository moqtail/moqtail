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
//! relay only through [`moqtail_probe::Inspect`].

mod access;
pub mod config;
mod connections;
mod control;
mod error;
mod list;
mod overview;
mod probe;
mod qlog;
mod routes;
mod snapshot;
mod tracks;

use anyhow::{Context, Result};
use config::{ApiConfig, Feature};
use control::ControlTracker;
use moqtail_probe::{ConnectionInfo, Inspect, Probe, TrackInfo};
use probe::{ApiProbe, Connection};
use snapshot::Cached;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};

/// What every handler can reach.
pub(crate) struct ApiState {
  pub config: ApiConfig,
  pub inspect: Arc<dyn Inspect>,
  pub probe: Arc<ApiProbe>,
  pub control: Arc<ControlTracker>,
  connections: Cached<Vec<ConnectionInfo>>,
  tracks: Cached<Vec<TrackInfo>>,
}

impl ApiState {
  pub(crate) fn new(config: ApiConfig, inspect: Arc<dyn Inspect>) -> Self {
    let control = Arc::new(ControlTracker::new(config.upstream_timeout));
    let probe = Arc::new(ApiProbe::new(
      config.has(Feature::State),
      config.closed_connection_history,
      control.clone(),
    ));
    ApiState {
      config,
      inspect,
      probe,
      control,
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

  /// Every track, at most a second old.
  pub async fn tracks(&self) -> Arc<Vec<TrackInfo>> {
    self.tracks.get(|| self.inspect.tracks()).await
  }
}

/// Binds the API's listener and serves it from a dedicated thread. Returns once the
/// socket is bound, so a port already in use is reported at relay startup. The
/// returned probe is the one the relay core should report to.
pub fn start(config: ApiConfig, inspect: Arc<dyn Inspect>) -> Result<Arc<dyn Probe>> {
  let listener = std::net::TcpListener::bind(config.bind)
    .with_context(|| format!("cannot bind the API to {}", config.bind))?;
  listener.set_nonblocking(true)?;
  let bind = config.bind;
  let state = Arc::new(ApiState::new(config, inspect));
  let probe: Arc<dyn Probe> = state.probe.clone();
  if state.config.has(Feature::State) {
    moqtail::transport::observe::set_control_observer(state.control.clone());
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
      runtime.block_on(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
          Ok(listener) => listener,
          Err(e) => {
            error!("API listener on {bind} failed: {e}");
            return;
          }
        };
        let app = routes::router(state).into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(listener, app).await {
          error!("API server on {bind} stopped: {e}");
        }
      });
    })?;
  info!("API listening on http://{bind}/api/v1");
  Ok(probe)
}

#[cfg(test)]
mod tests;
