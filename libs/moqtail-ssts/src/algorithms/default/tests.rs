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

//! Tests for the default allocation: the budget merge, and how a budget is
//! split across ranks, weights and saturation.

use super::*;
use crate::SetSnapshot;

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
fn budget_is_the_stricter_of_estimate_and_cap() {
  let no_streams = std::collections::HashMap::new();
  let input = |estimate, cap| AbrInput {
    client_id: 0,
    group_id: 0,
    bandwidth_estimate_kbps: estimate,
    configured_cap_kbps: cap,
    sets: &[],
    open_streams_per_set: &no_streams,
    stream_timeouts_since_last_decision: 0,
  };

  assert_eq!(input(1000, 500).budget_kbps(), 500);
  assert_eq!(input(300, 500).budget_kbps(), 300);
  // A zero estimate means "no estimate yet", not "no bandwidth".
  assert_eq!(input(0, 500).budget_kbps(), 500);
  // A zero cap means uncapped.
  assert_eq!(input(800, 0).budget_kbps(), 800);
  // Neither signal: unconstrained, rather than forwarding nothing.
  assert_eq!(input(0, 0).budget_kbps(), u64::MAX);
}

#[test]
fn single_set_picks_the_highest_affordable_track() {
  // Ladder 500/1000/2000 kbps with a 1500 kbps budget selects the 1000 kbps
  // track, which in this set has relay track id 1.
  let sets = [set(1, 0, 5, true, &[(500, 0), (1000, 1), (2000, 2)])];
  assert_eq!(allocate(1500, &sets)[&1], Some(1));
}

#[test]
fn two_sets_of_the_same_rank_split_by_weight() {
  // Weights 6:4 with a 3000 kbps budget: targets 1800 and 1200.
  // Set A ladder (800, 1600): selects 1600, which is its top.
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
fn a_saturated_set_gives_its_share_back() {
  // Set A (weight 1) tops out at 500; set B (weight 1) can use more.
  // First round: 1500/1500 of a 3000 kbps budget. A selects 500 (saturated),
  // B selects 1000 (not its top of 2000). A's unused 1000 goes to B:
  // target 2500, so B selects 2000.
  let sets = [
    set(1, 0, 1, true, &[(500, 10)]),
    set(2, 0, 1, true, &[(1000, 20), (2000, 21)]),
  ];
  let decisions = allocate(3000, &sets);
  assert_eq!(decisions[&1], Some(10));
  assert_eq!(decisions[&2], Some(21));
}

#[test]
fn a_high_rank_is_served_before_a_low_one() {
  // Rank 0 may consume as much as it needs before rank 1 sees anything.
  let sets = [
    set(1, 0, 1, true, &[(2000, 10)]),
    set(2, 1, 10, true, &[(500, 20), (1000, 21)]),
  ];
  let decisions = allocate(3000, &sets);
  assert_eq!(decisions[&1], Some(10));
  // 1000 kbps left over, so rank 1 selects its 1000 kbps track.
  assert_eq!(decisions[&2], Some(21));
}

#[test]
fn an_inactive_set_forwards_nothing() {
  let sets = [
    set(1, 0, 5, false, &[(500, 0), (1000, 1)]),
    set(2, 0, 5, true, &[(500, 2), (1000, 3)]),
  ];
  let decisions = allocate(10_000, &sets);
  assert_eq!(decisions[&1], None);
  assert_eq!(decisions[&2], Some(3));
}

#[test]
fn a_budget_below_the_lowest_tier_forwards_nothing() {
  let sets = [set(1, 0, 5, true, &[(500, 0), (1000, 1)])];
  assert_eq!(allocate(400, &sets)[&1], None);
}

#[test]
fn a_zero_weight_set_is_never_served() {
  // A set that asked for no share gets none, even when it is alone in its rank
  // tier; the budget is not returned to other tiers, because rank 0 is served
  // first and simply does not spend it here.
  let sets = [
    set(1, 0, 0, true, &[(500, 10), (4000, 11)]),
    set(2, 1, 1, true, &[(500, 20)]),
  ];
  let decisions = allocate(3000, &sets);
  assert_eq!(decisions[&1], None);
  assert_eq!(decisions[&2], Some(20));
}

#[test]
fn every_set_gets_an_answer() {
  // A set the selection leaves out would leave the relay without a decision to
  // forward on, so the allocation must cover all of them, active or not.
  let sets = [
    set(1, 3, 1, false, &[(500, 10)]),
    set(2, 1, 1, true, &[(500, 20)]),
    set(3, 1, 1, true, &[]),
  ];
  let decisions = allocate(10_000, &sets);
  assert_eq!(decisions.len(), 3);
  assert!(decisions.contains_key(&1));
  assert!(decisions.contains_key(&2));
  assert!(decisions.contains_key(&3));
}

#[test]
fn an_empty_set_list_decides_nothing() {
  assert!(allocate(10_000, &[]).is_empty());
}
