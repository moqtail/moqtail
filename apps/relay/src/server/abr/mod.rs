// Copyright 2026 The MOQtail Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use it except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! SSTS bandwidth allocation (draft-wilaw-moq-moqt-ssts).
//!
//! The algorithm trait, the per-decision snapshot it receives, and the
//! registry of the algorithms this relay ships.

pub mod backpressure;
pub mod default;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::server::client::MOQTClient;

/// The algorithms the relay implements and advertises in SETUP.
pub const SUPPORTED_SSTS_ALGORITHMS: &[u64] = &[0, 1];

/// Immutable copy of a switching set taken at decision time, so
/// algorithms run without holding the set manager's lock.
#[derive(Debug, Clone)]
pub struct SetSnapshot {
  pub id: u64,
  pub algorithm_id: u64,
  pub rank: u8,
  pub weight: u64,
  pub active: bool,
  /// (throughput threshold kbps, relay track id), ascending by threshold.
  pub members: Vec<(u64, u64)>,
}

/// A bandwidth allocation algorithm. Instances are shared across clients,
/// so per-client state must be keyed by the connection id.
pub trait AbrAlgorithm: Send + Sync {
  fn id(&self) -> u64;

  /// Decide, per set, which track to forward for `group_id`: `Some(track)`
  /// or `None` (forward nothing from the set).
  fn decide<'a>(
    &'a self,
    client: &'a Arc<MOQTClient>,
    group_id: u64,
    sets: &'a [SetSnapshot],
  ) -> Pin<Box<dyn Future<Output = HashMap<u64, Option<u64>>> + Send + 'a>>;

  /// A forwarding stream timed out on close and was reset; congestion
  /// evidence.
  fn on_stream_timeout(&self, _client_id: u64) {}
}

/// The algorithms this relay ships, in the order they are advertised.
pub(crate) fn algorithm_registry() -> Vec<Arc<dyn AbrAlgorithm>> {
  vec![
    Arc::new(default::DefaultAlgorithm),
    Arc::new(backpressure::BackpressureAlgorithm::new()),
  ]
}
