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

//! Tests for the tier state machine, including the two behaviours that are
//! surprising on purpose: summed depth across sets, and the fact that a tier
//! can be recovered as soon as the streams that caused the downshift are gone.

use std::collections::HashMap;

use crate::algorithms::backpressure::{BackpressureAlgorithm, BackpressureAlgorithmFactory};
use crate::{AbrAlgorithm, AbrAlgorithmFactory, AbrInput, SetSnapshot};

/// A set with a three-rung ladder: 500, 1000 and 2000 kbps.
fn set(id: u64, active: bool) -> SetSnapshot {
  SetSnapshot {
    id,
    algorithm_id: crate::algorithms::backpressure::BACKPRESSURE_ALGORITHM_ID,
    rank: 0,
    weight: 1,
    active,
    members: vec![(500, id * 10), (1000, id * 10 + 1), (2000, id * 10 + 2)],
  }
}

fn streams(entries: &[(u64, u64)]) -> HashMap<u64, u64> {
  entries.iter().copied().collect()
}

/// Run one group through the algorithm and return what the first set forwards.
fn decide_once(
  algorithm: &mut BackpressureAlgorithm,
  sets: &[SetSnapshot],
  open: &HashMap<u64, u64>,
  timeouts: u64,
  group: u64,
) -> Option<u64> {
  decide(&mut *algorithm, sets, open, timeouts, group)[&sets[0].id]
}

/// Run one group through the algorithm and return the whole selection.
fn decide(
  algorithm: &mut dyn AbrAlgorithm,
  sets: &[SetSnapshot],
  open: &HashMap<u64, u64>,
  timeouts: u64,
  group: u64,
) -> crate::Selection {
  let input = AbrInput {
    client_id: 7,
    group_id: group,
    bandwidth_estimate_kbps: 0,
    configured_cap_kbps: 0,
    sets,
    open_streams_per_set: open,
    stream_timeouts_since_last_decision: timeouts,
  };
  algorithm.decide(&input)
}

fn climb(algorithm: &mut BackpressureAlgorithm, sets: &[SetSnapshot], groups: usize) {
  let open = streams(&[(1, 1)]);
  for group in 0..groups {
    decide_once(algorithm, sets, &open, 0, group as u64);
  }
}

#[test]
fn starts_at_the_lowest_tier() {
  let sets = [set(1, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  // Depth one is the target, so nothing moves: the connection stays where it
  // started until it has shown it can hold a higher tier.
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 1)]), 0, 0),
    Some(10)
  );
}

#[test]
fn rises_one_tier_per_five_clear_groups() {
  let sets = [set(1, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  let clear = streams(&[(1, 1)]);

  for group in 0..4 {
    assert_eq!(
      decide_once(&mut algorithm, &sets, &clear, 0, group),
      Some(10),
      "tier should not move before the streak is complete"
    );
  }
  assert_eq!(
    decide_once(&mut algorithm, &sets, &clear, 0, 4),
    Some(11),
    "the fifth clear group raises the tier"
  );
  assert_eq!(
    decide_once(&mut algorithm, &sets, &clear, 0, 5),
    Some(11),
    "the streak restarts after an upshift"
  );
}

#[test]
fn falls_one_tier_at_the_downshift_depth() {
  let sets = [set(1, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  climb(&mut algorithm, &sets, 10);
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 1)]), 0, 10),
    Some(12),
    "two streaks should have reached the top of the ladder"
  );

  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 2)]), 0, 11),
    Some(11),
    "depth two drops one tier"
  );
}

#[test]
fn recovers_as_soon_as_the_streams_that_caused_the_drop_are_gone() {
  // The point of the stream accounting being live rather than cumulative: a
  // connection that drained its queue can climb again immediately, instead of
  // carrying the memory of a closed stream forever.
  let sets = [set(1, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  climb(&mut algorithm, &sets, 5);
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 1)]), 0, 5),
    Some(11)
  );

  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 2)]), 0, 6),
    Some(10),
    "congested: back to the bottom rung"
  );

  let drained = streams(&[]);
  for group in 7..11 {
    assert_eq!(
      decide_once(&mut algorithm, &sets, &drained, 0, group),
      Some(10),
      "counting up again"
    );
  }
  assert_eq!(
    decide_once(&mut algorithm, &sets, &drained, 0, 11),
    Some(11),
    "a drained queue is enough to climb again"
  );
}

#[test]
fn a_timeout_during_cooldown_drops_the_tier_again() {
  let sets = [set(1, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  climb(&mut algorithm, &sets, 15);
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 1)]), 0, 15),
    Some(12)
  );
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 2)]), 0, 16),
    Some(11),
    "first drop, cooldown starts"
  );
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 1)]), 1, 17),
    Some(10),
    "a stream that timed out on the tier just tried is enough to drop again"
  );
}

#[test]
fn rising_depth_during_cooldown_drops_the_tier_after_two_groups() {
  let sets = [set(1, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  climb(&mut algorithm, &sets, 15);
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 1)]), 0, 15),
    Some(12)
  );
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 2)]), 0, 16),
    Some(11)
  );
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 3)]), 0, 17),
    Some(11),
    "one deeper group is a transient, not a trend"
  );
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 4)]), 0, 18),
    Some(10),
    "two deeper groups in a row is a trend"
  );
}

#[test]
fn steady_depth_during_cooldown_never_drops_further() {
  let sets = [set(1, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  climb(&mut algorithm, &sets, 15);
  decide_once(&mut algorithm, &sets, &streams(&[(1, 2)]), 0, 15);
  for group in 16..25 {
    assert_eq!(
      decide_once(&mut algorithm, &sets, &streams(&[(1, 2)]), 0, group),
      Some(11),
      "depth that stopped rising should hold the tier"
    );
  }
}

#[test]
fn never_drops_below_the_lowest_tier() {
  let sets = [set(1, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  for group in 0..20 {
    assert_eq!(
      decide_once(&mut algorithm, &sets, &streams(&[(1, 9)]), group * 3, group),
      Some(10),
      "the bottom rung is all this algorithm can offer"
    );
  }
}

#[test]
fn an_inactive_set_forwards_nothing() {
  let sets = [set(1, false), set(2, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  let input = AbrInput {
    client_id: 7,
    group_id: 0,
    bandwidth_estimate_kbps: 0,
    configured_cap_kbps: 0,
    sets: &sets,
    open_streams_per_set: &streams(&[(1, 4)]),
    stream_timeouts_since_last_decision: 0,
  };
  let selection = algorithm.decide(&input);
  assert_eq!(selection[&1], None);
  assert_eq!(selection[&2], Some(20));
}

#[test]
fn every_active_set_moves_to_the_same_tier() {
  // One index for the whole connection: both sets change quality together,
  // which is what a subscriber with two tracks in two sets should perceive.
  let sets = [set(1, true), set(2, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  // Only set 1 has a stream in flight, so the summed depth stays at the target
  // and the tier can climb.
  let clear = streams(&[(1, 1)]);
  for group in 0..5 {
    decide_once(&mut algorithm, &sets, &clear, 0, group);
  }
  let selection = decide(&mut algorithm, &sets, &clear, 0, 5);
  // Both sets have the same ladder, so the same tier means the same rung.
  assert_eq!(selection[&1], Some(11));
  assert_eq!(selection[&2], Some(21));
}

#[test]
fn two_active_sets_are_pinned_to_the_lowest_tier() {
  // One forwarding stream per set is normal operation, not congestion. Depth
  // is summed across the connection's active sets, though, so two sets already
  // read as DOWNSHIFT_DEPTH: the tier can neither rise (a clear streak needs
  // depth at or below DEPTH_TARGET) nor fall below the rung it starts on. A
  // connection with two switching sets therefore stays on the bottom rung
  // however healthy the link is.
  //
  // This is a known limitation of the ported thresholds, which were written
  // for a single set; the fix is to scale them with the number of active sets.
  // Recorded as a test so the behaviour cannot be mistaken for a regression.
  let sets = [set(1, true), set(2, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  let one_each = streams(&[(1, 1), (2, 1)]);
  for group in 0..30 {
    assert_eq!(
      decide_once(&mut algorithm, &sets, &one_each, 0, group),
      Some(10),
      "group {group}: summed depth keeps the tier pinned to the bottom rung"
    );
  }
}

#[test]
fn the_tier_is_capped_by_the_longest_ladder() {
  let sets = [set(1, true)];
  let mut algorithm = BackpressureAlgorithm::new();
  // Far more clear groups than the ladder has rungs.
  climb(&mut algorithm, &sets, 40);
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 1)]), 0, 40),
    Some(12),
    "the top rung, not an index past it"
  );
  assert_eq!(
    decide_once(&mut algorithm, &sets, &streams(&[(1, 2)]), 0, 41),
    Some(11),
    "a downshift from the clamped index still lands on a real rung"
  );
}

#[test]
fn instances_do_not_share_state() {
  // The old implementation kept one state machine per client id in a shared
  // map; instances must now be independent, which is what makes the contract
  // usable from a single-threaded decision loop without a lock.
  let sets = [set(1, true)];
  let factory = BackpressureAlgorithmFactory;
  let mut first = factory.create();
  let mut second = factory.create();

  let clear = streams(&[(1, 1)]);
  let input = AbrInput {
    client_id: 7,
    group_id: 0,
    bandwidth_estimate_kbps: 0,
    configured_cap_kbps: 0,
    sets: &sets,
    open_streams_per_set: &clear,
    stream_timeouts_since_last_decision: 0,
  };
  for _ in 0..20 {
    first.decide(&input);
  }
  assert_eq!(
    second.decide(&input)[&1],
    Some(10),
    "fresh instance, bottom rung"
  );
}

#[test]
fn cooldown_brackets_the_settle_down_period() {
  // The state machine on its own, without sets or tracks: cooldown is what
  // stops one congested group from cascading the tier to the bottom.
  use super::state::{Observation, TierState};

  let mut state = TierState::default();
  for _ in 0..5 {
    state.observe(Observation::new(1, 0));
  }
  assert_eq!(state.tier(), 1);
  assert!(!state.in_cooldown());

  state.observe(Observation::new(2, 0));
  assert_eq!(state.tier(), 0);
  assert!(state.in_cooldown(), "a downshift starts the cooldown");

  state.observe(Observation::new(1, 0));
  assert!(!state.in_cooldown(), "depth at the target ends it");
}

#[test]
fn a_timeout_counts_as_depth_and_as_evidence() {
  use super::state::Observation;
  let observation = Observation::new(1, 2);
  assert_eq!(
    observation.depth, 3,
    "a timed-out stream counts like an open one"
  );
  assert!(observation.timed_out);
  assert!(!Observation::new(1, 0).timed_out);
}
