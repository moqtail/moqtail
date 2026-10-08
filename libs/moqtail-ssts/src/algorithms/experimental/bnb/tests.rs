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

//! Tests for the budget algorithm: the backlog arithmetic, the budget that gives
//! it a unit, the timeout channel, the drain window, and the fact that all of it
//! is per set.

use std::collections::HashMap;

use super::BnbAlgorithm;
use super::state::BnbParams;
use crate::AbrAlgorithm;
use crate::AbrInput;
use crate::DecisionReason;
use crate::SetSnapshot;
use crate::StreamSnapshot;

/// A ladder of `rungs` members for one set, with the subscriber buffer it
/// negotiated.
fn set(id: u64, buffer_ms: u64, rungs: usize, active: bool) -> SetSnapshot {
  SetSnapshot {
    id,
    algorithm_id: super::BNB_ALGORITHM_ID,
    rank: 0,
    weight: 1,
    active,
    members: (0..rungs)
      .map(|rung| (300 + 500 * rung as u64, id * 100 + rung as u64))
      .collect(),
    subscriber_buffer_ms: buffer_ms,
    algorithm_params: vec![],
    group_span_ms: 0,
  }
}

/// Everything one decision looks at, so a test can name the state of the
/// connection rather than assemble it from four arguments.
struct Scenario {
  sets: Vec<SetSnapshot>,
  streams: Vec<StreamSnapshot>,
  timeouts: HashMap<u64, u64>,
}

impl Scenario {
  fn new(sets: Vec<SetSnapshot>) -> Self {
    Self {
      sets,
      streams: Vec::new(),
      timeouts: HashMap::new(),
    }
  }

  /// `depth` open streams on `set_id`, all `age_ms` old, on distinct groups.
  fn with_streams(mut self, set_id: u64, depth: u64, age_ms: u64) -> Self {
    for index in 0..depth {
      self.streams.push(StreamSnapshot {
        set_id,
        relay_track_id: set_id * 100,
        group_id: index,
        age_ms,
        resent: false,
      });
    }
    self
  }

  fn with_timeout(mut self, set_id: u64, count: u64) -> Self {
    *self.timeouts.entry(set_id).or_default() += count;
    self
  }

  fn input(&self, group_id: u64, now_ms: u64) -> AbrInput<'_> {
    AbrInput {
      client_id: 7,
      group_id,
      bandwidth_estimate_kbps: 0,
      configured_cap_kbps: 0,
      reason: DecisionReason::GroupBoundary,
      now_ms,
      sets: &self.sets,
      streams: &self.streams,
      stream_timeouts_since_last_decision: &self.timeouts,
    }
  }
}

/// Run the same scenario over successive groups and report what set `id`
/// forwards each time. The clock continues from `start_ms`: the drain window is
/// a duration, and a test that restarted the clock at zero would hold tiers for
/// a window that had already passed.
fn run(
  algorithm: &mut BnbAlgorithm,
  scenario: &Scenario,
  set_id: u64,
  groups: u64,
  start_ms: u64,
  step_ms: u64,
) -> Vec<Option<u64>> {
  (0..groups)
    .map(|group| {
      let input = scenario.input(group, start_ms + group * step_ms);
      algorithm.decide(&input)[&set_id]
    })
    .collect()
}

/// One rung's worth of time after a change at `at_ms`.
fn after_drain(at_ms: u64) -> u64 {
  at_ms + BnbParams::default().drain_window_ms + 1
}

#[test]
fn a_set_without_a_subscriber_forwards_nothing() {
  let scenario = Scenario::new(vec![set(1, 1000, 3, false)]);
  let mut algorithm = BnbAlgorithm::new();
  assert_eq!(
    algorithm.decide(&scenario.input(0, 0)).tracks[&1],
    None,
    "nobody is watching, so nothing goes"
  );
}

#[test]
fn the_queue_climbs_after_the_streak_of_healthy_groups() {
  // Buffer 1000 ms, no backlog at all: the climb is pure streak.
  let scenario = Scenario::new(vec![set(1, 1000, 4, true)]);
  let mut algorithm = BnbAlgorithm::new();

  let served = run(&mut algorithm, &scenario, 1, 4, 0, 4000);
  // The third healthy group completes the streak of three; the rung then walks
  // one step per three groups, because each climb resets the streak.
  assert_eq!(
    served,
    vec![Some(100), Some(100), Some(101), Some(101)],
    "one rung per three healthy groups"
  );
}

#[test]
fn a_growing_queue_vetoes_the_climb() {
  let mut algorithm = BnbAlgorithm::new();

  // One more stream open each group: the queue is accumulating, which is the
  // veto on climbing (and, past the safe line, the trigger to drop).
  for group in 0..9 {
    let scenario = Scenario::new(vec![set(1, 1000, 4, true)]).with_streams(1, group + 1, 0);
    let served = algorithm
      .decide(&scenario.input(group, group * 4000))
      .tracks[&1];
    assert_eq!(
      served,
      Some(100),
      "the queue grew in group {group}, so the rung stays put"
    );
  }
}

#[test]
fn a_backlog_past_the_danger_fraction_drops_a_rung() {
  // Climb to rung 1 first, then a backlog of 800 ms against a 1000 ms buffer:
  // 80% occupied, past the 70% danger line, and a discard is imminent.
  let healthy = Scenario::new(vec![set(1, 1000, 4, true)]);
  let mut algorithm = BnbAlgorithm::new();
  run(&mut algorithm, &healthy, 1, 3, 0, 4000);
  assert_eq!(algorithm.tier(1, 3), 1);

  let congested = Scenario::new(vec![set(1, 1000, 4, true)]).with_streams(1, 1, 800);
  let served = algorithm.decide(&congested.input(3, after_drain(8000)))[&1];
  assert_eq!(served, Some(100), "the backlog is past the danger fraction");
}

/// The unit the algorithm exists to have: a stream age only means something
/// against the group period, because a healthy group's stream lives exactly one
/// period.
#[test]
fn the_backlog_is_the_age_beyond_one_group_period() {
  // Same 800 ms of stream age, twice: with a 700 ms group period it is 100 ms
  // of backlog (healthy), with no measured period it reads as 800 ms (danger).
  let mut with_span = set(1, 1000, 4, true);
  with_span.group_span_ms = 700;
  let scenario = Scenario::new(vec![with_span]).with_streams(1, 1, 800);
  let mut algorithm = BnbAlgorithm::new();
  // Four groups of the same depth: the queue is flat, so the streak climbs the
  // rung, and the stream age is the only thing that could argue otherwise.
  run(&mut algorithm, &scenario, 1, 4, 0, 4000);
  let climbed = algorithm.tier(1, 3);
  assert_eq!(climbed, 1, "a flat queue with a healthy backlog climbs");

  algorithm.decide(&scenario.input(4, after_drain(12_000)));
  assert_eq!(
    algorithm.tier(1, 3),
    climbed,
    "a stream one group period old is a healthy pipeline, not a backlog"
  );

  // The same connection with no measured group period: the age has nothing to
  // be healthy against, so it reads as a danger-sized backlog and the set never
  // accumulates the streak it would need to climb.
  let without_span = Scenario::new(vec![set(1, 1000, 4, true)]).with_streams(1, 1, 800);
  let mut algorithm = BnbAlgorithm::new();
  let served = run(&mut algorithm, &without_span, 1, 4, 0, 4000);
  assert_eq!(
    algorithm.tier(1, 3),
    0,
    "the same age, with no measured period to subtract, reads as a backlog: {served:?}"
  );
}

#[test]
fn no_negotiated_buffer_means_no_danger() {
  // A subscriber that negotiated no buffer has given the relay no permission to
  // discard, and so has no deadline for the algorithm to be late for.
  let scenario = Scenario::new(vec![set(1, 0, 4, true)]).with_streams(1, 1, 60_000);
  let mut algorithm = BnbAlgorithm::new();

  let served = run(&mut algorithm, &scenario, 1, 4, 0, 4000);
  assert_eq!(
    served.last(),
    Some(&Some(101)),
    "with no budget the streak is the only signal, and it says climb: {served:?}"
  );
}

#[test]
fn timeouts_veto_a_climb_and_sustained_timeouts_drop_a_rung() {
  let mut algorithm = BnbAlgorithm::new();

  // One timeout: the veto blocks the climb but is not itself a downshift.
  let vetoed = Scenario::new(vec![set(1, 1000, 4, true)]).with_timeout(1, 1);
  let served = run(&mut algorithm, &vetoed, 1, 2, 0, 4000);
  assert!(
    served.iter().all(|track| *track == Some(100)),
    "a vetoed set does not climb: {served:?}"
  );

  // Climb it once with clean groups: the veto decays and the streak rebuilds.
  let healthy = Scenario::new(vec![set(1, 1000, 4, true)]);
  run(&mut algorithm, &healthy, 1, 9, 20_000, 4000);
  let tier = algorithm.tier(1, 3);
  assert!(tier > 0, "the healthy run should have climbed, got {tier}");

  // Three timeouts in one group: the rung is unsustainable whatever the queue
  // says, so it drops on its own.
  let timeouts = Scenario::new(vec![set(1, 1000, 4, true)]).with_timeout(1, 3);
  algorithm.decide(&timeouts.input(20, after_drain(60_000)));
  assert_eq!(
    algorithm.tier(1, 3),
    tier - 1,
    "sustained timeouts drop a rung on their own"
  );
}

#[test]
fn the_timeout_veto_decays_after_clean_groups() {
  let mut algorithm = BnbAlgorithm::new();

  let veto = Scenario::new(vec![set(1, 1000, 4, true)]).with_timeout(1, 1);
  algorithm.decide(&veto.input(0, 0));
  assert_eq!(algorithm.decide(&veto.input(1, 4000))[&1], Some(100));

  // The veto decays one per clean group, so the streak can rebuild once it has
  // run down.
  let healthy = Scenario::new(vec![set(1, 1000, 4, true)]);
  let served = run(&mut algorithm, &healthy, 1, 7, 20_000, 4000);
  // Two of the seven groups are spent letting the veto run down; the rest
  // rebuild the streak, one climb per three of them.
  assert_eq!(
    served.last(),
    Some(&Some(102)),
    "the veto is gone and the streak rebuilt: {served:?}"
  );
}

#[test]
fn the_rung_holds_across_the_drain_window() {
  let mut algorithm = BnbAlgorithm::with_params(BnbParams::default());

  // Climb once, at t = 8000.
  let healthy = Scenario::new(vec![set(1, 1000, 4, true)]);
  run(&mut algorithm, &healthy, 1, 3, 0, 4000);
  assert_eq!(algorithm.tier(1, 3), 1);

  // Immediately after the change, a danger-sized backlog cannot move the rung:
  // the sample measures the streams that were already in flight. The climb
  // happened at t = 8000.
  let danger = Scenario::new(vec![set(1, 1000, 4, true)]).with_streams(1, 1, 900);
  algorithm.decide(&danger.input(3, 8100));
  assert_eq!(
    algorithm.tier(1, 3),
    1,
    "inside the drain window the tier is held"
  );

  // Once the window has passed, the same backlog acts.
  algorithm.decide(&danger.input(4, after_drain(8000)));
  assert_eq!(
    algorithm.tier(1, 3),
    0,
    "after the window the danger signal is believed again"
  );
}

#[test]
fn a_sustained_timeout_overrides_the_drain_window() {
  let params = BnbParams::default();
  let mut algorithm = BnbAlgorithm::with_params(params);

  let healthy = Scenario::new(vec![set(1, 1000, 4, true)]);
  run(&mut algorithm, &healthy, 1, 3, 0, 4000);
  let tier = algorithm.tier(1, 3);

  // A timeout burst straight after the change: the hold is for the queue's
  // benefit, not an excuse to keep serving a rung that is losing streams.
  let timeouts = Scenario::new(vec![set(1, 1000, 4, true)]).with_timeout(1, 3);
  algorithm.decide(&timeouts.input(3, 10));
  assert_eq!(
    algorithm.tier(1, 3),
    tier - 1,
    "the emergency still acts inside the drain window"
  );
}

/// The reason this algorithm has a rung per set while the depth algorithms have
/// one per connection: two sets with different buffers are in different
/// situations.
#[test]
fn each_set_moves_on_its_own_evidence() {
  let scenario = Scenario::new(vec![
    SetSnapshot {
      ..set(1, 1000, 3, true)
    },
    SetSnapshot {
      ..set(2, 1000, 3, true)
    },
  ])
  .with_streams(2, 1, 900);
  let mut algorithm = BnbAlgorithm::new();

  // Climb both to rung 1 first.
  let healthy = Scenario::new(vec![set(1, 1000, 3, true), set(2, 1000, 3, true)]);
  run(&mut algorithm, &healthy, 1, 3, 0, 4000);
  assert_eq!(algorithm.tier(1, 2), 1);
  assert_eq!(algorithm.tier(2, 2), 1);

  algorithm.decide(&scenario.input(3, 30_000));
  assert_eq!(algorithm.tier(1, 2), 1, "set 1 has no backlog of its own");
  assert_eq!(algorithm.tier(2, 2), 0, "set 2 does");
}

#[test]
fn a_timeout_on_one_set_does_not_veto_another() {
  let scenario =
    Scenario::new(vec![set(1, 1000, 4, true), set(2, 1000, 4, true)]).with_timeout(1, 1);
  let mut algorithm = BnbAlgorithm::new();

  let served = run(&mut algorithm, &scenario, 2, 3, 0, 4000);
  assert_eq!(
    served.last(),
    Some(&Some(201)),
    "set 2 has seen no timeouts and climbs on its own evidence"
  );
  assert_eq!(algorithm.tier(1, 3), 0, "set 1 is still vetoed");
}

#[test]
fn the_rung_never_drops_below_the_bottom() {
  let scenario = Scenario::new(vec![set(1, 1000, 3, true)])
    .with_streams(1, 4, 90_000)
    .with_timeout(1, 9);
  let mut algorithm = BnbAlgorithm::new();

  for group in 0..10 {
    algorithm.decide(&scenario.input(group, group * 10_000 + 2000));
  }
  assert_eq!(algorithm.tier(1, 2), 0);
}

/// Sets come and go on a long-lived connection; their state must go with them.
#[test]
fn a_set_that_disappears_takes_its_state_with_it() {
  let mut algorithm = BnbAlgorithm::new();

  let healthy = Scenario::new(vec![set(1, 1000, 4, true)]);
  run(&mut algorithm, &healthy, 1, 9, 0, 4000);
  assert!(algorithm.tier(1, 3) > 1);

  // A decision about a different set, then back to the first one.
  let other = Scenario::new(vec![set(2, 1000, 4, true)]);
  algorithm.decide(&other.input(0, 100_000));

  let served = algorithm.decide(&healthy.input(1, 200_000));
  assert_eq!(
    served.tracks[&1],
    Some(100),
    "the set was rebuilt from scratch rather than resuming a stale climb"
  );
}

#[test]
fn the_thresholds_come_from_the_host() {
  // A host that wants a more cautious algorithm: danger at 30% of the buffer
  // instead of 70%.
  let tuning = crate::Tuning::from_pairs(["bnb.danger_factor=0.3", "bnb.down_streak=1"]);
  let params = BnbParams::from_tuning(&tuning);
  assert_eq!(params.danger_factor, 0.3);
  assert_eq!(
    params.safe_factor, 0.2,
    "untouched keys keep their defaults"
  );

  let mut algorithm = BnbAlgorithm::with_params(params);
  let healthy = Scenario::new(vec![set(1, 1000, 4, true)]);
  run(&mut algorithm, &healthy, 1, 3, 0, 4000);
  assert_eq!(algorithm.tier(1, 3), 1);

  // 400 ms of backlog against a 1000 ms buffer is 40%: danger under this
  // host's tuning, harmless under the built-in 70%.
  let congested = Scenario::new(vec![set(1, 1000, 4, true)]).with_streams(1, 1, 400);
  algorithm.decide(&congested.input(3, 20_000));
  assert_eq!(algorithm.tier(1, 3), 0);
}
