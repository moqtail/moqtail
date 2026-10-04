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

//! The per-connection decision loop: collect the input, run the algorithms
//! this connection negotiated, validate what came back, publish it for the
//! forward gate.
//!
//! This is the mechanism side of SSTS. The decisions themselves live in
//! `moqtail-ssts`; what happens here is the translation between the two: the
//! relay's state becomes an [`AbrInput`], and an algorithm's [`Selection`]
//! becomes a per-group decision the gate in `subscription.rs` can act on.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moqtail_ssts::registry::registry;
use moqtail_ssts::{AbrAlgorithm, AbrInput, Selection, SetSnapshot};
use tokio::select;
use tracing::warn;

use crate::server::client::MOQTClient;
use crate::server::config::AppConfig;

use super::AbrMessage;

/// How often the decision is re-evaluated without a new group. The answer for
/// the group in flight cannot change.
const DECISION_INTERVAL: Duration = Duration::from_millis(100);

/// Number of groups of decisions to keep behind the live edge, so a publisher
/// running behind still gets a decision instead of forwarding unselected.
const DECISION_WINDOW: u64 = 5;

/// An algorithm this connection runs, with the identity it was created from so
/// that a bad decision can be attributed in the log.
struct Running {
  id: u64,
  name: &'static str,
  algorithm: Box<dyn AbrAlgorithm>,
}

/// Spawn the SSTS decision loop for a connection that has switching sets.
pub(crate) fn start(client: Arc<MOQTClient>) {
  tokio::spawn(async move {
    // Claiming the receiver is what makes starting twice a no-op: the second
    // task finds nothing to read from and leaves.
    let Some(mut rx) = client.ssts.abr_rx.lock().await.take() else {
      return;
    };

    let client_id = client.connection_id as u64;
    let mut running: Vec<Running> = Vec::new();
    for id in &client.ssts.negotiated_algorithms {
      match registry().get(*id) {
        Some(factory) => running.push(Running {
          id: *id,
          name: factory.name(),
          algorithm: factory.create(),
        }),
        None => warn!(
          client_id,
          algorithm_id = id,
          "SSTS: negotiated algorithm is not registered; its sets will forward nothing"
        ),
      }
    }
    if running.is_empty() {
      return;
    }

    let mut ticker = tokio::time::interval(DECISION_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // The group the periodic re-evaluation runs for.
    let mut last_group: Option<u64> = None;
    // The largest group ever seen. Decisions are pruned against this rather
    // than against the group being decided, so an Object that arrives late on
    // an old group cannot prune the decisions of the live edge.
    let mut largest_group: u64 = 0;
    // Streams reset for delivery timeout since the previous decision: the
    // congestion evidence the algorithms ask for. It is read only by this
    // connection's algorithms, so it lives here instead of in shared state.
    let mut stream_timeouts: u64 = 0;

    loop {
      select! {
        msg = rx.recv() => {
          match msg {
            Some(AbrMessage::NewGroup(group_id)) => {
              last_group = Some(group_id.max(last_group.unwrap_or(0)));
              largest_group = largest_group.max(group_id);
              // One boundary per group.
              if client.ssts.has_group_decision(group_id).await {
                // The answer already exists; wake anyone waiting on it
                client.ssts.decision_notify.notify_waiters();
                continue;
              }
              decide(&mut running, &client, group_id, largest_group, &mut stream_timeouts).await;
            }
            Some(AbrMessage::StreamTimeout { group_id }) => {
              largest_group = largest_group.max(group_id);
              stream_timeouts += 1;
              if let Some(group_id) = last_group {
                decide(&mut running, &client, group_id, largest_group, &mut stream_timeouts).await;
              }
            }
            None => return,
          }
        }
        _ = ticker.tick() => {
          if let Some(group_id) = last_group {
            decide(&mut running, &client, group_id, largest_group, &mut stream_timeouts).await;
          }
        }
        _ = client.connection.closed() => {
          return;
        }
      }
    }
  });
}

/// Run every algorithm over the sets assigned to it, for `group_id`.
async fn decide(
  running: &mut [Running],
  client: &Arc<MOQTClient>,
  group_id: u64,
  largest_group: u64,
  stream_timeouts: &mut u64,
) {
  let (sets, open_streams_per_set) = client.ssts.decision_snapshot().await;
  if sets.is_empty() {
    return;
  }

  let bandwidth_estimate_kbps = client.connection.bandwidth_estimate_kbps();
  let configured_cap_kbps = AppConfig::load().write_kbps_limit;
  // Taken once and handed to every algorithm: they all decide at this moment,
  // so they all see the same evidence.
  let timeouts = std::mem::take(stream_timeouts);

  let mut decisions: HashMap<u64, Option<u64>> = HashMap::new();
  for algorithm in running.iter_mut() {
    let algorithm_sets: Vec<SetSnapshot> = sets
      .iter()
      .filter(|set| set.algorithm_id == algorithm.id)
      .cloned()
      .collect();
    if algorithm_sets.is_empty() {
      continue;
    }

    let input = AbrInput {
      client_id: client.connection_id as u64,
      group_id,
      bandwidth_estimate_kbps,
      configured_cap_kbps,
      sets: &algorithm_sets,
      open_streams_per_set: &open_streams_per_set,
      stream_timeouts_since_last_decision: timeouts,
    };
    decisions.extend(validate(
      algorithm.name,
      algorithm.algorithm.decide(&input),
      &algorithm_sets,
    ));
  }

  // A set no running algorithm owns gets an explicit "forward nothing".
  // Leaving it out of the map would stall the gate on that group instead.
  let unowned: Vec<&SetSnapshot> = sets
    .iter()
    .filter(|set| !decisions.contains_key(&set.id))
    .collect();
  for set in unowned {
    warn!(
      algorithm_id = set.algorithm_id,
      set_id = set.id,
      "SSTS: no running algorithm owns this set; forwarding nothing"
    );
    decisions.insert(set.id, None);
  }

  // A group already being delivered keeps its answer: recording is insert-if-absent
  let recorded = client
    .ssts
    .record_group_decision(
      group_id,
      decisions,
      largest_group.saturating_sub(DECISION_WINDOW),
    )
    .await;

  if recorded {
    // No epoch bump: answers are final, so recording one makes no cached
    // verdict stale; waiters wake on the notify.
    client.ssts.decision_notify.notify_waiters();
  }
}

/// Check an algorithm's answer against the sets it was shown, so a buggy or
/// hostile algorithm degrades visibly instead of silently blackholing a track.
///
/// Every set in `sets` gets an entry in the result: the gate waits for a
/// decision per set, so an omitted one would block that group forever.
fn validate(algorithm: &str, selection: Selection, sets: &[SetSnapshot]) -> Selection {
  let mut validated = Selection::with_capacity(sets.len());

  for set in sets {
    match selection.get(&set.id) {
      Some(Some(track_id)) if set.has_track(*track_id) => {
        validated.insert(set.id, Some(*track_id));
      }
      Some(Some(track_id)) => {
        warn!(
          algorithm,
          set_id = set.id,
          relay_track_id = track_id,
          "SSTS: algorithm selected a track that is not a member of the set; forwarding nothing"
        );
        validated.insert(set.id, None);
      }
      Some(None) => {
        validated.insert(set.id, None);
      }
      None => {
        warn!(
          algorithm,
          set_id = set.id,
          "SSTS: algorithm returned no decision for a set; forwarding nothing"
        );
        validated.insert(set.id, None);
      }
    }
  }

  let invented: Vec<u64> = selection
    .keys()
    .filter(|set_id| !sets.iter().any(|set| set.id == **set_id))
    .copied()
    .collect();
  if !invented.is_empty() {
    warn!(
      algorithm,
      ?invented,
      "SSTS: algorithm returned sets it was not given; ignored"
    );
  }

  validated
}
