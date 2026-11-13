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
mod error;
mod routes;

use anyhow::{Context, Result};
use config::ApiConfig;
use moqtail_probe::Inspect;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};

/// What every handler can reach.
pub(crate) struct ApiState {
  pub config: ApiConfig,
  pub inspect: Arc<dyn Inspect>,
}

/// Binds the API's listener and serves it from a dedicated thread. Returns once the
/// socket is bound, so a port already in use is reported at relay startup.
pub fn start(config: ApiConfig, inspect: Arc<dyn Inspect>) -> Result<()> {
  let listener = std::net::TcpListener::bind(config.bind)
    .with_context(|| format!("cannot bind the API to {}", config.bind))?;
  listener.set_nonblocking(true)?;
  let bind = config.bind;
  let state = Arc::new(ApiState { config, inspect });

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
  Ok(())
}

#[cfg(test)]
mod tests;
