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

pub mod backpressure;
pub mod default;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::server::client::{AbrMessage, MOQTClient};

/// The algorithms the relay implements and advertises in SETUP.
pub const SUPPORTED_SSTS_ALGORITHMS: &[u64] = &[0, 1];

/// Periodic re-evaluation interval: bandwidth estimates and stream depth
/// drift between groups.
const TICK_MS: u64 = 100;

/// close_stream gives up on a graceful finish after this long and resets
/// the stream, so an abandoned group stops occupying a stream slot.
const DISCARD_TIMEOUT_MS: u64 = 1600;

const DECISION_WINDOW: u64 = 5;

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

fn algorithm_registry() -> Vec<Arc<dyn AbrAlgorithm>> {
  vec![
    Arc::new(default::DefaultAlgorithm),
    Arc::new(backpressure::BackpressureAlgorithm::new()),
  ]
}

pub(crate) fn start_abr_controller(client: Arc<MOQTClient>) {
  let client_id = client.connection_id as u64;
  let algorithms = algorithm_registry();
  tokio::spawn(async move {
    let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started once");

    client
      .discard_timeout_ms
      .store(DISCARD_TIMEOUT_MS, Ordering::Relaxed);

    let mut tick = tokio::time::interval(Duration::from_millis(TICK_MS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut last_group: Option<u64> = None;

    loop {
      tokio::select! {
          msg = abr_rx.recv() => {
              match msg {
                  Some(AbrMessage::NewGroup(group_id)) => {
                      last_group = Some(group_id);
                      decide(&client, &algorithms, group_id).await;
                  }
                  Some(AbrMessage::StreamTimeout { group_id }) => {
                      debug!(
                          client_id,
                          group_id,
                          "ABR: stream timeout on close — forwarding to algorithms"
                      );
                      for alg in &algorithms {
                          alg.on_stream_timeout(client_id);
                      }
                  }
                  None => break,
              }
          }

          _ = tick.tick() => {
              if let Some(group_id) = last_group {
                  decide(&client, &algorithms, group_id).await;
              }
          }

          _ = client.connection.closed() => {
              info!(client_id, "ABR controller shutting down: connection physically closed");
              break;
          }
      }
    }
  });
}

async fn decide(client: &Arc<MOQTClient>, algorithms: &[Arc<dyn AbrAlgorithm>], group_id: u64) {
  let sets: Vec<SetSnapshot> = {
    let manager = client.switching_sets.read().await;
    manager
      .sets
      .values()
      .map(|s| SetSnapshot {
        id: s.id,
        algorithm_id: s.algorithm_id,
        rank: s.rank,
        weight: s.weight,
        active: s.is_active(),
        members: s
          .members
          .iter()
          .map(|m| (m.throughput_threshold_kbps, m.relay_track_id))
          .collect(),
      })
      .collect()
  };

  if sets.is_empty() {
    return;
  }

  let mut by_algorithm: HashMap<u64, Vec<SetSnapshot>> = HashMap::new();
  for set in sets {
    by_algorithm.entry(set.algorithm_id).or_default().push(set);
  }

  let mut decisions: HashMap<u64, Option<u64>> = HashMap::new();
  for (algorithm_id, alg_sets) in &by_algorithm {
    match algorithms.iter().find(|a| a.id() == *algorithm_id) {
      Some(alg) => {
        decisions.extend(alg.decide(client, group_id, alg_sets).await);
      }
      None => {
        // Defensive: subscribe time already rejects unsupported algorithms.
        warn!(
          algorithm_id,
          "SSTS: unsupported algorithm; forwarding nothing"
        );
        for set in alg_sets {
          decisions.insert(set.id, None);
        }
      }
    }
  }

  let changed = {
    let mut group_decisions = client.group_decisions.write().await;
    let changed = group_decisions.get(&group_id) != Some(&decisions);
    if changed {
      group_decisions.insert(group_id, decisions.clone());
      group_decisions.retain(|&k, _| k >= group_id.saturating_sub(DECISION_WINDOW));
    }
    changed
  };

  if changed {
    info!(
      group_id,
      selections = %format_selections(&decisions),
      "SSTS: bandwidth allocation"
    );
    client.decision_notify.notify_waiters();
  }
}

fn format_selections(decisions: &HashMap<u64, Option<u64>>) -> String {
  let mut pairs: Vec<(&u64, &Option<u64>)> = decisions.iter().collect();
  pairs.sort();
  pairs
    .iter()
    .map(|(set, track)| {
      format!(
        "set {} -> {}",
        set,
        track
          .map(|t| format!("{t}"))
          .unwrap_or_else(|| "none".to_string())
      )
    })
    .collect::<Vec<_>>()
    .join(", ")
}
