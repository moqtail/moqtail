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

//! The weighted, strict-priority split of a bandwidth budget across sets.

use crate::{Selection, SetSnapshot};

/// Split `budget_kbps` across `sets`: per set, the relay track id of the member
/// to forward, or `None` when the set forwards nothing this group (it has no
/// subscriber, or the allocation did not reach its lowest tier).
pub(super) fn allocate(budget_kbps: u64, sets: &[SetSnapshot]) -> Selection {
  let mut decisions: Selection = Selection::new();
  let mut remaining = budget_kbps;

  // Strict priority: every set of a rank is served before the next rank sees
  // anything, so the rank ladder is walked from the highest priority down.
  let mut ranks: Vec<u8> = sets.iter().map(|set| set.rank).collect();
  ranks.sort();
  ranks.dedup();

  for rank in ranks {
    let tier: Vec<&SetSnapshot> = sets
      .iter()
      .filter(|set| set.rank == rank && set.active)
      .collect();
    if tier.is_empty() {
      continue;
    }

    // selection[i] is the (threshold, relay track id) tier[i] ended up with.
    let mut selection: Vec<Option<(u64, u64)>> = vec![None; tier.len()];
    // Sets still competing for the tier pool; saturated ones leave it.
    let mut contending: Vec<usize> = (0..tier.len()).collect();
    let mut tier_pool = remaining;

    loop {
      let total_weight: u64 = contending
        .iter()
        .map(|&i| tier[i].weight)
        .sum::<u64>()
        .max(1);

      for &i in &contending {
        let target = tier_pool.saturating_mul(tier[i].weight) / total_weight;
        selection[i] = tier[i]
          .members
          .iter()
          .rev()
          .find(|&&(threshold, _)| threshold <= target)
          .copied();
      }

      // A set that selected its top track cannot use more of the pool, so it
      // leaves it and the rest get its unused share.
      let saturated: Vec<usize> = contending
        .iter()
        .copied()
        .filter(|&i| {
          matches!(
            (selection[i], tier[i].top_threshold_kbps()),
            (Some((threshold, _)), Some(top)) if threshold == top
          )
        })
        .collect();

      if saturated.is_empty() {
        break;
      }

      for &i in &saturated {
        if let Some((threshold, _)) = selection[i] {
          tier_pool = tier_pool.saturating_sub(threshold);
        }
      }
      contending.retain(|i| !saturated.contains(i));
    }

    for (i, set) in tier.iter().enumerate() {
      decisions.insert(set.id, selection[i].map(|(_, track_id)| track_id));
      remaining =
        remaining.saturating_sub(selection[i].map(|(threshold, _)| threshold).unwrap_or(0));
    }
  }

  // A set with no subscriber forwards nothing, and would take its rank tier's
  // share of the budget with it if it were left in the split above.
  for set in sets {
    decisions.entry(set.id).or_default();
  }

  decisions
}
