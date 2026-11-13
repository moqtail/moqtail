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

//! The relay's side of [`moqtail_probe::Inspect`]: plain copies of its state for
//! the observability layer.

use super::Server;
use moqtail_probe::{Inspect, RelayStatus};
use std::sync::atomic::Ordering;
use std::time::UNIX_EPOCH;

pub(crate) struct RelayInspect(pub Server);

impl Inspect for RelayInspect {
  fn status(&self) -> RelayStatus {
    let server = &self.0;
    RelayStatus {
      version: env!("MOQTAIL_VERSION").to_string(),
      pid: std::process::id(),
      started_at_ms: server
        .started_at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0),
      accepting: server.accepting.load(Ordering::Relaxed),
      draining: server.draining.load(Ordering::Relaxed),
    }
  }

  fn config(&self) -> serde_json::Value {
    // The tokens are marked to be skipped when serializing, so they never appear here.
    serde_json::to_value(self.0.app_config).unwrap_or_default()
  }
}
