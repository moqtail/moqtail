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

//! Connects the relay core to its observability layer at startup. This is the only
//! module of the relay that may use `relay_api`; the test below keeps it that way.

use crate::server::Server;
use crate::server::inspect::RelayInspect;
use anyhow::Result;
use relay_api::config::{ApiConfig, ApiSettings};
use std::sync::Arc;
use tracing::warn;

/// Starts the HTTP API when `api_enable` is set. Errors in the `api_*` settings stop
/// the relay at startup rather than leaving it running without the API.
pub fn start(server: &Server) -> Result<()> {
  let c = server.app_config;
  if !c.api_enable {
    return Ok(());
  }
  let (config, warnings) = ApiConfig::from_settings(ApiSettings {
    bind: c.api_bind,
    features: c.api_features.clone(),
    allow: c.api_allow.clone(),
    read_token: c.api_read_token.clone(),
    write_token: c.api_write_token.clone(),
    metrics_bind: c.api_metrics_bind,
    metrics_token: c.api_metrics_token.clone(),
    cors_origins: c.api_cors_origins.clone(),
    closed_connection_history: c.closed_connection_history,
    qlog_folder: c.qlog_folder.as_ref().map(Into::into),
    upstream_timeout: c.upstream_subscribe_timeout,
    latency_detail: Some(c.latency_detail.clone()),
    latency_sample: c.latency_sample,
    slowlog_threshold: c.slowlog_threshold,
    slowlog_first_object_threshold: c.slowlog_first_object_threshold,
    slowlog_size: c.slowlog_size,
    accounting_zones: c.accounting_zones.clone(),
  })?;
  for warning in warnings {
    warn!("{warning}");
  }
  let probe = relay_api::start(config, Arc::new(RelayInspect(server.clone())))?;
  moqtail_probe::install(probe);
  Ok(())
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  /// The relay core reaches the observability layer only through `moqtail_probe`.
  /// Everything under `src/server` is core, so none of it may name the API crate or
  /// the libraries it is built on.
  #[test]
  fn core_does_not_import_the_observability_layer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![root.join("server.rs")];
    let mut dirs = vec![root.join("server")];
    while let Some(dir) = dirs.pop() {
      for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
          dirs.push(path);
        } else if path.extension().is_some_and(|e| e == "rs") {
          files.push(path);
        }
      }
    }
    for file in files {
      let text = std::fs::read_to_string(&file).unwrap();
      for forbidden in ["relay_api", "axum", "prometheus"] {
        assert!(
          !text.contains(forbidden),
          "{} mentions `{forbidden}`; core files must reach the observability layer \
           only through moqtail_probe",
          file.display()
        );
      }
    }
  }
}
