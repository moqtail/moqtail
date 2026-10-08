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

//! Tests for the exploration phase: what makes it fast, what ends it, and the
//! fact that it ends for good.

use std::collections::HashMap;

use crate::AbrAlgorithm;
use crate::AbrInput;
use crate::DecisionReason;
use crate::SetSnapshot;
use crate::StreamSnapshot;
use crate::algorithms::asd::AsdAlgorithm;
use crate::algorithms::asd_v11::AsdV11Algorithm;

/// A set with a five-rung ladder. The tests count groups against it, so the
/// rungs are named by index: tier 0 is track `id*10`, tier 4 is `id*10+4`.
fn set(id: u64, active: bool) -> SetSnapshot {
  SetSnapshot {
    id,
    algorithm_id: super::ASD_V11_ALGORITHM_ID,
    rank: 0,
    weight: 1,
    active,
    members: vec![
      (300, id * 10),
      (600, id * 10 + 1),
      (1000, id * 10 + 2),
      (1500, id * 10 + 3),
      (2000, id * 10 + 4),
    ],
    subscriber_buffer_ms: 0,
    algorithm_params: vec![],
    group_span_ms: 0,
  }
}

/// The timeout evidence, all of it on set 1 (the tests use one set).
fn timeout_evidence(count: u64) -> HashMap<u64, u64> {
  if count == 0 {
    HashMap::new()
  } else {
    [(1, count)].into_iter().collect()
  }
}

fn streams(entries: &[(u64, u64)]) -> Vec<StreamSnapshot> {
  let mut table = Vec::new();
  for (set_id, count) in entries {
    for index in 0..*count {
      table.push(StreamSnapshot {
        set_id: *set_id,
        relay_track_id: set_id * 10,
        group_id: index,
        age_ms: 0,
        resent: false,
      });
    }
  }
  table
}

/// One group at the given depth and timeout count, returning what the first set
/// of `sets` forwards.
fn group(
  algorithm: &mut dyn AbrAlgorithm,
  sets: &[SetSnapshot],
  depth: u64,
  timeouts: u64,
  group_id: u64,
) -> Option<u64> {
  let live = streams(&[(1, depth)]);
  let input = AbrInput {
    client_id: 7,
    group_id,
    bandwidth_estimate_kbps: 0,
    configured_cap_kbps: 0,
    reason: DecisionReason::GroupBoundary,
    now_ms: group_id * 1000,
    sets,
    streams: &live,
    stream_timeouts_since_last_decision: &timeout_evidence(timeouts),
  };
  algorithm.decide(&input).tracks[&sets[0].id]
}

#[test]
fn exploration_climbs_one_tier_per_clean_group() {
  let sets = [set(1, true)];
  let mut algorithm = AsdV11Algorithm::new();

  // The first clean group already moves: a connection with bandwidth it is not
  // using has no reason to wait five groups to start finding out.
  assert_eq!(group(&mut algorithm, &sets, 1, 0, 0), Some(11));
  assert_eq!(group(&mut algorithm, &sets, 1, 0, 1), Some(12));
  assert_eq!(group(&mut algorithm, &sets, 1, 0, 2), Some(13));
}

/// The point of the algorithm, stated against the thing it was built from: the
/// same evidence, four times the climbing speed at the start.
#[test]
fn the_same_clean_run_climbs_faster_than_plain_asd() {
  let sets = [set(1, true)];
  let mut exploring = AsdV11Algorithm::new();
  let mut cautious = AsdAlgorithm::new();

  for group_id in 0..4 {
    group(&mut exploring, &sets, 1, 0, group_id);
    group(&mut cautious, &sets, 1, 0, group_id);
  }
  assert_eq!(group(&mut exploring, &sets, 1, 0, 4), Some(14));
  assert_eq!(
    group(&mut cautious, &sets, 1, 0, 4),
    Some(11),
    "plain ASD has completed one five-group streak"
  );
}

#[test]
fn congestion_ends_the_exploration_and_the_cautious_streak_takes_over() {
  let sets = [set(1, true)];
  let mut algorithm = AsdV11Algorithm::new();

  group(&mut algorithm, &sets, 1, 0, 0);
  group(&mut algorithm, &sets, 1, 0, 1);
  assert!(algorithm.exploring());

  // Depth two: the tier falls, and the exploration is over.
  assert_eq!(group(&mut algorithm, &sets, 2, 0, 2), Some(11));
  assert!(!algorithm.exploring());

  // The group that gets back to the target only leaves cooldown; then the full
  // five-group streak is what climbs again, not the exploration's one group.
  assert_eq!(group(&mut algorithm, &sets, 1, 0, 3), Some(11));
  assert_eq!(group(&mut algorithm, &sets, 1, 0, 4), Some(11));
  assert_eq!(group(&mut algorithm, &sets, 1, 0, 5), Some(11));
  assert_eq!(group(&mut algorithm, &sets, 1, 0, 6), Some(11));
  assert_eq!(
    group(&mut algorithm, &sets, 1, 0, 7),
    Some(12),
    "the fifth clean group after the congestion is what climbs"
  );
}

#[test]
fn reaching_the_top_ends_the_exploration_without_overshooting() {
  let sets = [set(1, true)];
  let mut algorithm = AsdV11Algorithm::new();

  for group_id in 0..4 {
    group(&mut algorithm, &sets, 1, 0, group_id);
  }
  // The fourth clean group puts the tier on the top rung (track 14 of a set
  // whose rungs are 10, 11, 12, 13, 14), and that is the end of exploring:
  // there is nothing above it to find.
  assert_eq!(group(&mut algorithm, &sets, 1, 0, 4), Some(14));
  assert!(
    !algorithm.exploring(),
    "the top of the ladder ends the exploration"
  );
  assert_eq!(
    group(&mut algorithm, &sets, 1, 0, 5),
    Some(14),
    "and the tier stays there rather than drifting above the ladder"
  );
}

/// A set whose tracks are still arriving has a ladder that is growing, so
/// reaching its current top is not evidence that exploring is finished.
#[test]
fn an_inactive_set_ends_neither_the_exploration_nor_the_selection() {
  let paused = [set(1, false)];
  let mut algorithm = AsdV11Algorithm::new();

  for group_id in 0..5 {
    assert_eq!(
      group(&mut algorithm, &paused, 0, 0, group_id),
      None,
      "a set with no subscriber forwards nothing"
    );
  }
  assert!(
    algorithm.exploring(),
    "no subscriber means no evidence about the path at all"
  );

  let live = [set(1, true)];
  assert_eq!(group(&mut algorithm, &live, 1, 0, 5), Some(14));
}

/// The exploration is a phase of one connection, not a cycle: a connection
/// knocked back to the bottom does not get to sprint up again.
#[test]
fn the_exploration_does_not_restart() {
  let sets = [set(1, true)];
  let mut algorithm = AsdV11Algorithm::new();

  // Congest it straight away: the exploration ends before it has climbed.
  assert_eq!(group(&mut algorithm, &sets, 3, 0, 0), Some(10));
  assert!(!algorithm.exploring());

  // A long clean run at the bottom rung moves once per five groups, which is
  // the cautious walk, not one tier per group.
  let climbed: Vec<Option<u64>> = (1..=6)
    .map(|group_id| group(&mut algorithm, &sets, 1, 0, group_id))
    .collect();
  assert_eq!(
    climbed,
    vec![Some(10), Some(10), Some(10), Some(10), Some(11), Some(11),],
    "the second climb is the cautious one"
  );
}

#[test]
fn a_stream_timeout_ends_the_exploration() {
  let sets = [set(1, true)];
  let mut algorithm = AsdV11Algorithm::new();

  group(&mut algorithm, &sets, 1, 0, 0);
  // Depth one plus one stream the transport gave up on: two, which is the
  // congestion threshold.
  assert_eq!(group(&mut algorithm, &sets, 1, 1, 1), Some(10));
  assert!(!algorithm.exploring());
}

#[test]
fn one_index_serves_sets_of_different_heights() {
  let short = SetSnapshot {
    id: 2,
    members: vec![(400, 99)],
    ..set(2, true)
  };
  let sets = [set(1, true), short];
  let mut algorithm = AsdV11Algorithm::new();

  for group_id in 0..3 {
    let live = streams(&[(1, 1)]);
    let input = AbrInput {
      client_id: 7,
      group_id,
      bandwidth_estimate_kbps: 0,
      configured_cap_kbps: 0,
      reason: DecisionReason::GroupBoundary,
      now_ms: 0,
      sets: &sets,
      streams: &live,
      stream_timeouts_since_last_decision: &HashMap::new(),
    };
    let selection = algorithm.decide(&input);
    assert_eq!(
      selection.tracks[&2],
      Some(99),
      "the short set serves its only member whatever the index says"
    );
  }
}
