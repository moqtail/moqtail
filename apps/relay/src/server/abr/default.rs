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

//! The default SSTS bandwidth allocation algorithm (id 0,
//! draft-wilaw-moq-moqt-ssts, Section 6.3.1).
//!
//! Strict priority: `rank` totally orders the sets, and a set receives its
//! full allocation before any lower-priority (higher-rank) set receives
//! any. `weight` only splits a rank tier between sets; a saturated set
//! (top track selected) gives its unused share back to the remaining
//! same-rank sets, repeatedly, until the tier is fully claimed or every
//! set in it selected its top track.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::{AbrAlgorithm, MOQTClient, SetSnapshot};

/// "Unlimited" bandwidth: used when neither the transport nor the config
/// yields an estimate, so the highest-throughput track is always selected.
const UNLIMITED_KBPS: u64 = u64::MAX / 4;

/// Algorithm 0: the spec's default bandwidth allocation.
pub struct DefaultAlgorithm;

impl AbrAlgorithm for DefaultAlgorithm {
  fn id(&self) -> u64 {
    0
  }

  fn decide<'a>(
    &'a self,
    client: &'a Arc<MOQTClient>,
    _group_id: u64,
    sets: &'a [SetSnapshot],
  ) -> Pin<Box<dyn Future<Output = HashMap<u64, Option<u64>>> + Send + 'a>> {
    Box::pin(async move {
      let cap_kbps = crate::server::config::AppConfig::load().write_kbps_limit;
      let b_total = effective_bandwidth_kbps(client.connection.bandwidth_estimate_kbps(), cap_kbps);
      allocate(b_total, sets)
    })
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

/// Allocate `b_total` across the sets: per set, the selected track's relay
/// track id, or `None` when nothing from the set is forwarded (inactive,
/// or allocation below the lowest threshold).
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
            (selected[i], tier[i].members.last().map(|m| m.0)),
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
      algorithm_id: 0,
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
    let sets = [
      set(1, 0, 6, true, &[(800, 10), (1600, 11)]),
      set(2, 0, 4, true, &[(500, 20), (1000, 21), (2000, 22)]),
    ];
    let decisions = allocate(3000, &sets);
    assert_eq!(decisions[&1], Some(11));
    assert_eq!(decisions[&2], Some(21));
  }

  #[test]
  fn test_saturated_set_share_is_reallocated() {
    // Set A (weight 1) tops out at 500; set B (weight 1) can use more.
    // First round: 1500/1500 of a 3000 budget. A selects 500 (saturated),
    // B selects 1000 (not its top of 2000). A's unused 1000 goes to B:
    // target 2500 -> B selects 2000.
    let sets = [
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
    let sets = [
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
    let sets = [
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
