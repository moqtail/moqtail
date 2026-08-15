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

//! SSTS bandwidth allocation (draft-wilaw-moq-moqt-ssts, Section 6.3.1).
//!
//! Per new group and periodically, allocates the subscriber's bandwidth
//! across its switching sets and records the one track to forward per set
//! (`group_decisions[group][set]`), or `None` when nothing is forwarded.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tracing::info;

use crate::server::client::AbrMessage;
use crate::server::client::MOQTClient;

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Periodic re-evaluation interval; B_total drifts between groups.
const TICK_MS: u64 = 100;

/// close_stream gives up on a graceful finish after this long and resets the
/// stream, so an abandoned group stops occupying a stream slot.
const DISCARD_TIMEOUT_MS: u64 = 1600;

/// "Unlimited" bandwidth: used when neither the transport nor the config
/// yields an estimate, so the highest-throughput track is always selected.
const UNLIMITED_KBPS: u64 = u64::MAX / 4;

/// How many groups' decisions are kept.
const DECISION_WINDOW: u64 = 5;

// ─────────────────────────────────────────────────────────────────────────────
// Main controller task
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn start_abr_controller(client: Arc<MOQTClient>) {
  let client_id = client.connection_id as u64;
  tokio::spawn(async move {
    let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started once");

    client
      .discard_timeout_ms
      .store(DISCARD_TIMEOUT_MS, Ordering::Relaxed);

    let cap_kbps = crate::server::config::AppConfig::load().write_kbps_limit;

    let mut tick = tokio::time::interval(Duration::from_millis(TICK_MS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut last_group: Option<u64> = None;

    loop {
      tokio::select! {
          msg = abr_rx.recv() => {
              let Some(AbrMessage::NewGroup(group_id)) = msg else { break };
              last_group = Some(group_id);
              decide(&client, group_id, cap_kbps).await;
          }

          _ = tick.tick() => {
              // Re-run for the current group: the estimate may have moved.
              if let Some(group_id) = last_group {
                  decide(&client, group_id, cap_kbps).await;
              }
          }

          _ = client.connection.closed() => {
              info!(client_id = %client_id, "ABR controller shutting down: connection physically closed");
              break;
          }
      }
    }
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Bandwidth allocation (Section 6.3.1)
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot of a switching set at decision time.
struct SetSnapshot {
  id: u64,
  rank: u8,
  weight: u64,
  active: bool,
  /// (throughput threshold, relay track id), ascending by threshold.
  members: Vec<(u64, u64)>,
}

impl SetSnapshot {
  fn top_threshold(&self) -> Option<u64> {
    self.members.last().map(|(t, _)| *t)
  }
}

/// Combine the transport's bandwidth estimate with the configured cap.
fn effective_bandwidth_kbps(transport_kbps: u64, cap_kbps: u64) -> u64 {
  match (transport_kbps, cap_kbps) {
    (t, c) if t > 0 && c > 0 => t.min(c),
    (t, _) if t > 0 => t,
    (_, c) if c > 0 => c,
    _ => UNLIMITED_KBPS,
  }
}

/// Run the allocation algorithm and store the per-set decisions for `group_id`;
/// inactive sets get `None` (no data forwarded).
async fn decide(client: &Arc<MOQTClient>, group_id: u64, cap_kbps: u64) {
  let sets: Vec<SetSnapshot> = {
    let manager = client.switching_sets.read().await;
    manager
      .sets
      .values()
      .map(|s| SetSnapshot {
        id: s.id,
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

  let b_total = effective_bandwidth_kbps(client.connection.bandwidth_estimate_kbps(), cap_kbps);

  let decisions = allocate(b_total, &sets);

  // Store the decision; only wake waiters when something changed.
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
      b_total_kbps = b_total,
      selections = %format_selections(&decisions),
      "SSTS: bandwidth allocation"
    );
    client.decision_notify.notify_waiters();
  }
}

/// The Section 6.3.1 allocation: sets are served in rank order (lower rank
/// first), same-rank sets split the tier by weight, and a saturated set's
/// unused share is redistributed among its same-rank peers.
/// Returns the selected track per set, or `None` when nothing is forwarded.
fn allocate(b_total: u64, sets: &[SetSnapshot]) -> HashMap<u64, Option<u64>> {
  let mut decisions: HashMap<u64, Option<u64>> = HashMap::new();
  let mut b_remaining = b_total;

  // Strict priority: each distinct rank is served before the next.
  let mut ranks: Vec<u8> = sets.iter().map(|s| s.rank).collect();
  ranks.sort();
  ranks.dedup();

  for rank in ranks {
    let tier: Vec<&SetSnapshot> = sets.iter().filter(|s| s.rank == rank && s.active).collect();
    if tier.is_empty() {
      continue;
    }

    // selected[i] = (threshold, relay track id) of tier[i]'s selection.
    let mut selected: Vec<Option<(u64, u64)>> = tier.iter().map(|_| None).collect();
    let mut active: Vec<usize> = (0..tier.len()).collect();
    let mut tier_pool = b_remaining;

    loop {
      let sum_weight: u64 = active.iter().map(|&i| tier[i].weight).sum::<u64>().max(1);

      for &i in &active {
        let target = tier_pool.saturating_mul(tier[i].weight) / sum_weight;
        // Highest-throughput track whose threshold the allocation covers.
        selected[i] = tier[i]
          .members
          .iter()
          .rev()
          .find(|&&(threshold, _)| threshold <= target)
          .copied();
      }

      // A set is saturated when it selected its highest-available track.
      let saturated: HashSet<usize> = active
        .iter()
        .copied()
        .filter(|&i| {
          matches!(
            (selected[i], tier[i].top_threshold()),
            (Some((threshold, _)), Some(top)) if threshold == top
          )
        })
        .collect();

      if saturated.is_empty() {
        break;
      }

      // Saturated sets keep their selection; their unused share of the tier
      // pool is redistributed among the remaining sets.
      for &i in &active {
        if saturated.contains(&i)
          && let Some((threshold, _)) = selected[i]
        {
          tier_pool = tier_pool.saturating_sub(threshold);
        }
      }
      active.retain(|&i| !saturated.contains(&i));
    }

    for (i, set) in tier.iter().enumerate() {
      decisions.insert(set.id, selected[i].map(|(_, track_id)| track_id));
      b_remaining =
        b_remaining.saturating_sub(selected[i].map(|(threshold, _)| threshold).unwrap_or(0));
    }
  }

  // Inactive sets forward nothing.
  for set in sets {
    if !set.active {
      decisions.insert(set.id, None);
    }
  }

  decisions
}

fn format_selections(decisions: &HashMap<u64, Option<u64>>) -> String {
  let mut pairs: Vec<(u64, Option<u64>)> = decisions.iter().map(|(k, v)| (*k, *v)).collect();
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_effective_bandwidth() {
    assert_eq!(effective_bandwidth_kbps(1000, 500), 500);
    assert_eq!(effective_bandwidth_kbps(300, 500), 300);
    assert_eq!(effective_bandwidth_kbps(0, 500), 500);
    assert_eq!(effective_bandwidth_kbps(800, 0), 800);
    assert_eq!(effective_bandwidth_kbps(0, 0), UNLIMITED_KBPS);
  }

  fn set(id: u64, rank: u8, weight: u64, active: bool, members: &[(u64, u64)]) -> SetSnapshot {
    SetSnapshot {
      id,
      rank,
      weight,
      active,
      members: members.to_vec(),
    }
  }

  #[test]
  fn test_single_set_picks_highest_affordable_track() {
    // Ladder 500/1000/2000 kbps with a 1500 kbps budget selects the 1000 kbps
    // track (relay track id 2).
    let sets = vec![set(1, 0, 5, true, &[(500, 0), (1000, 1), (2000, 2)])];
    let decisions = allocate(1500, &sets);
    assert_eq!(decisions[&1], Some(1));
  }

  #[test]
  fn test_two_sets_same_rank_split_by_weight() {
    // Weights 6:4 with a 3000 kbps budget: targets 1800 and 1200.
    // Set A ladder (800, 1600): selects 1600 (saturated, its top).
    // Set B ladder (500, 1000, 2000): selects 1000.
    let sets = vec![
      set(1, 0, 6, true, &[(800, 10), (1600, 11)]),
      set(2, 0, 4, true, &[(500, 20), (1000, 21), (2000, 22)]),
    ];
    let decisions = allocate(3000, &sets);
    assert_eq!(decisions[&1], Some(11));
    assert_eq!(decisions[&2], Some(21));
  }

  #[test]
  fn test_saturated_set_share_is_reallocated() {
    // 3000 budget, weights 1:1. A tops out at 500 (saturated); its unused
    // share goes to B, whose target grows to 2500 -> it selects 2000.
    let sets = vec![
      set(1, 0, 1, true, &[(500, 10)]),
      set(2, 0, 1, true, &[(1000, 20), (2000, 21)]),
    ];
    let decisions = allocate(3000, &sets);
    assert_eq!(decisions[&1], Some(10));
    assert_eq!(decisions[&2], Some(21));
  }

  #[test]
  fn test_strict_priority_across_ranks() {
    // Rank 0 set may consume as much as it needs before rank 1 sees anything.
    let sets = vec![
      set(1, 0, 1, true, &[(2000, 10)]),
      set(2, 1, 10, true, &[(500, 20), (1000, 21)]),
    ];
    let decisions = allocate(3000, &sets);
    assert_eq!(decisions[&1], Some(10));
    // 1000 kbps left over: rank 1 selects 1000.
    assert_eq!(decisions[&2], Some(21));
  }

  #[test]
  fn test_inactive_set_forwards_nothing() {
    let sets = vec![
      set(1, 0, 5, false, &[(500, 0), (1000, 1)]),
      set(2, 0, 5, true, &[(500, 2), (1000, 3)]),
    ];
    let decisions = allocate(10_000, &sets);
    assert_eq!(decisions[&1], None);
    assert_eq!(decisions[&2], Some(3));
  }

  #[test]
  fn test_no_track_affordable_forwards_nothing() {
    // Budget below the lowest threshold of the set.
    let sets = vec![set(1, 0, 5, true, &[(500, 0), (1000, 1)])];
    let decisions = allocate(400, &sets);
    assert_eq!(decisions[&1], None);
  }
}
