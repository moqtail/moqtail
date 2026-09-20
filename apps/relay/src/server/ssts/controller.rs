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

//! The SSTS controller: one task per client that negotiated SSTS.
//!
//! It is the only place that runs the bandwidth allocation algorithms: it
//! collects the signals, builds the per-decision snapshot, hands it to one
//! algorithm instance per set's algorithm, validates what comes back, caches
//! the result per group, and wakes the forward gate.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::server::abr::{AbrAlgorithm, SetSnapshot};
use crate::server::client::MOQTClient;
use crate::server::ssts::AbrMessage;

/// Periodic re-evaluation interval: bandwidth estimates and stream depth
/// drift between groups.
const TICK_MS: u64 = 100;

const DECISION_WINDOW: u64 = 5;

pub(crate) fn start(client: Arc<MOQTClient>) {
  let client_id = client.connection_id as u64;
  let algorithms = crate::server::abr::algorithm_registry();
  tokio::spawn(async move {
    let mut abr_rx = client
      .ssts
      .abr_rx
      .lock()
      .await
      .take()
      .expect("ABR started once");

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
    let manager = client.ssts.switching_sets.read().await;
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
    let mut group_decisions = client.ssts.group_decisions.write().await;
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
    client.ssts.decision_notify.notify_waiters();
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
