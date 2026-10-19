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

//! Tests for the three things wasd changes: where the depth is measured, when
//! it is read, and — the one thing the versions differ in — how far a downshift
//! reaches back into the groups it overtook.

use std::collections::HashMap;

use crate::AbrAlgorithm;
use crate::AbrInput;
use crate::DecisionReason;
use crate::SetSnapshot;
use crate::StreamSnapshot;
use crate::algorithms::wasd::WasdAlgorithm;
use crate::algorithms::wasd::WasdParams;

/// A set with a four-rung ladder. The rungs are addressed by index, so tier
/// `n` of set `id` is track `id * 10 + n`.
fn set(id: u64, active: bool) -> SetSnapshot {
  SetSnapshot {
    id,
    algorithm_id: super::WASD_ALGORITHM_ID,
    rank: 0,
    weight: 1,
    active,
    members: vec![
      (300, id * 10),
      (600, id * 10 + 1),
      (1000, id * 10 + 2),
      (1500, id * 10 + 3),
    ],
    subscriber_buffer_ms: 0,
    algorithm_params: vec![],
    group_span_ms: 0,
  }
}

/// The same set with the subscriber's reorder buffer filled in, which is the
/// budget a re-send inherits the remainder of.
fn set_with_buffer(id: u64, subscriber_buffer_ms: u64) -> SetSnapshot {
  SetSnapshot {
    subscriber_buffer_ms,
    ..set(id, true)
  }
}

/// A set with a chosen ladder, buffer and group cadence: the three numbers
/// the dynamic upshift threshold `U(k)` reads.
fn ladder_set(
  id: u64,
  bitrates: &[u64],
  subscriber_buffer_ms: u64,
  group_span_ms: u64,
) -> SetSnapshot {
  SetSnapshot {
    id,
    algorithm_id: super::WASD_ALGORITHM_ID,
    rank: 0,
    weight: 1,
    active: true,
    members: bitrates
      .iter()
      .enumerate()
      .map(|(k, &b)| (b, id * 10 + k as u64))
      .collect(),
    subscriber_buffer_ms,
    algorithm_params: vec![],
    group_span_ms,
  }
}

/// The track a set serves at `tier`.
fn track_at(set_id: u64, tier: usize) -> u64 {
  set_id * 10 + tier as u64
}

/// A live stream of `set_id` carrying `track`.
fn stream(set_id: u64, track: u64, group: u64) -> StreamSnapshot {
  StreamSnapshot {
    set_id,
    relay_track_id: track,
    group_id: group,
    age_ms: 0,
    resent: false,
  }
}

/// A live stream of `set_id` carrying `track`, `age_ms` into its delivery.
fn stream_with_age(set_id: u64, track: u64, group: u64, age_ms: u64) -> StreamSnapshot {
  StreamSnapshot {
    age_ms,
    ..stream(set_id, track, group)
  }
}

/// The same, marked as a re-send of an overtaken group.
fn resent(set_id: u64, track: u64, group: u64) -> StreamSnapshot {
  StreamSnapshot {
    resent: true,
    ..stream(set_id, track, group)
  }
}

/// The timeout evidence for a connection whose transport has given up on nothing.
fn no_timeouts() -> &'static HashMap<u64, u64> {
  static EMPTY: std::sync::OnceLock<HashMap<u64, u64>> = std::sync::OnceLock::new();
  EMPTY.get_or_init(HashMap::new)
}

fn input<'a>(
  sets: &'a [SetSnapshot],
  streams: &'a [StreamSnapshot],
  reason: DecisionReason,
  group_id: u64,
) -> AbrInput<'a> {
  AbrInput {
    client_id: 1,
    group_id,
    bandwidth_estimate_kbps: 0,
    configured_cap_kbps: 0,
    sets,
    reason,
    now_ms: 0,
    streams,
    stream_timeouts_since_last_decision: no_timeouts(),
  }
}

/// The tier the algorithm is serving, read off what set 1 forwards.
fn tier_of(selection: &crate::Selection, set_id: u64) -> usize {
  match selection[&set_id] {
    Some(track) => (track % 10) as usize,
    None => panic!("set {set_id} was expected to forward something"),
  }
}

/// `groups` clean boundary readings: the exploration climbs one tier per group.
fn climb(algorithm: &mut WasdAlgorithm, sets: &[SetSnapshot], groups: u64) -> usize {
  let mut tier = 0;
  for group in 0..groups {
    let empty: Vec<StreamSnapshot> = Vec::new();
    let selection = algorithm.decide(&input(sets, &empty, DecisionReason::GroupBoundary, group));
    tier = tier_of(&selection, 1);
  }
  tier
}

#[test]
fn the_exploration_climbs_one_tier_per_clean_group() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::new();

  for expected in 1usize..=3 {
    let empty: Vec<StreamSnapshot> = Vec::new();
    let selection = algorithm.decide(&input(
      &sets,
      &empty,
      DecisionReason::GroupBoundary,
      expected as u64,
    ));
    assert_eq!(tier_of(&selection, 1), expected);
  }
}

/// The whole point of measuring at one rung: streams left behind by an earlier
/// quality are not this tier's congestion.
#[test]
fn the_depth_counts_only_the_streams_at_the_current_tier() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::new();
  // Two clean groups: the tier is 2, and six streams are in flight at other
  // rungs of the same set.
  climb(&mut algorithm, &sets, 2);

  let elsewhere = [
    stream(1, track_at(1, 0), 7),
    stream(1, track_at(1, 0), 8),
    stream(1, track_at(1, 3), 9),
    stream(1, track_at(1, 3), 10),
    stream(1, track_at(1, 1), 11),
    stream(1, track_at(1, 1), 12),
  ];
  let selection = algorithm.decide(&input(&sets, &elsewhere, DecisionReason::GroupBoundary, 20));
  assert_eq!(
    tier_of(&selection, 1),
    3,
    "six streams at qualities the algorithm has already rejected are not depth"
  );

  // The same count at the tier being delivered is congestion, and the tier
  // falls: two streams of the delivered tier still open when a group's first
  // Object arrives is two groups behind, and the boundary needs no streak to
  // know it.
  let here = [stream(1, track_at(1, 3), 13), stream(1, track_at(1, 3), 14)];
  let selection = algorithm.decide(&input(&sets, &here, DecisionReason::GroupBoundary, 21));
  assert_eq!(tier_of(&selection, 1), 2);
}

/// A re-send is the relay's own doing. Counting it would let a downshift report
/// the congestion it was meant to relieve, and the ladder would walk itself down
/// on the strength of the relay's bookkeeping.
#[test]
fn a_resent_stream_is_not_depth() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::new();
  climb(&mut algorithm, &sets, 2);
  let tier = 2;

  let re_sends = [
    resent(1, track_at(1, tier), 5),
    resent(1, track_at(1, tier), 6),
  ];
  let selection = algorithm.decide(&input(&sets, &re_sends, DecisionReason::GroupBoundary, 30));
  assert_eq!(
    tier_of(&selection, 1),
    3,
    "the streams a downshift caused do not call for another one"
  );

  let fresh = [stream(1, track_at(1, 3), 5), stream(1, track_at(1, 3), 6)];
  let selection = algorithm.decide(&input(&sets, &fresh, DecisionReason::GroupBoundary, 31));
  assert_eq!(
    tier_of(&selection, 1),
    2,
    "the same two streams, at the tier now being served, are"
  );
}

/// The descent waits for real congestion, not for its own repair: after a
/// step from tier 2 to tier 1, the re-sent group still open at the tier now
/// served is the downshift's own doing. Counting it walked the ladder one
/// rung further per repair - three rungs for a half-second dip on the
/// measured profile - so the second step must come from fresh streams, or
/// not come at all.
#[test]
fn a_repair_in_flight_is_not_a_second_congestion() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::new();
  climb(&mut algorithm, &sets, 2);

  // Step one: a congested boundary at tier 2 falls to tier 1.
  let congested = [stream(1, track_at(1, 2), 30), stream(1, track_at(1, 2), 31)];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 32));
  assert_eq!(tier_of(&selection, 1), 1, "the first step");

  // The repair: group 31 re-sent at tier 1, still open when group 33's
  // boundary is read next to group 32's fresh stream at the same tier.
  // Only the fresh stream is depth: one group in flight is on target.
  let in_flight = [resent(1, track_at(1, 1), 31), stream(1, track_at(1, 1), 32)];
  let selection = algorithm.decide(&input(&sets, &in_flight, DecisionReason::GroupBoundary, 33));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the repair does not call for the next step"
  );
}

#[test]
fn a_v3_downshift_asks_to_overtake_the_sets_it_moved_down() {
  let sets = [set(1, true), set(2, false)];
  let mut algorithm = WasdAlgorithm::v3();
  climb(&mut algorithm, &sets, 2);

  let congested = [stream(1, track_at(1, 2), 4), stream(1, track_at(1, 2), 5)];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 40));
  assert!(
    selection.supersede.contains_key(&1),
    "the groups still in flight above the new rung were overtaken"
  );
  assert!(
    selection.supersede[&1].age_staged,
    "the re-send is staged by age: the newest overtaken group at the new rung, the older ones one below"
  );
  assert!(
    !selection.supersede[&1].newest_only,
    "and it reaches every overtaken group, not just the newest"
  );
  assert!(
    !selection.supersede.contains_key(&2),
    "a set with no subscriber has no delivery to overtake"
  );
}

/// The one thing v2 is: the request stops at the newest overtaken group.
#[test]
fn a_v2_downshift_reaches_only_the_newest_overtaken_group() {
  let sets = [set_with_buffer(1, 800)];
  let mut algorithm = WasdAlgorithm::v2();
  climb(&mut algorithm, &sets, 2);

  let congested = [
    stream_with_age(1, track_at(1, 2), 4, 500),
    stream_with_age(1, track_at(1, 2), 5, 250),
  ];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 40));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the tier fell, as in every version"
  );
  assert!(
    selection.supersede.contains_key(&1),
    "the newest overtaken group is named"
  );
  assert!(
    selection.supersede[&1].newest_only,
    "and the older ones are left to finish where they are"
  );
  assert!(
    !selection.supersede[&1].age_staged,
    "there is no age to stage when one group is reached"
  );
  assert_eq!(
    selection.resend_deadline_ms(1),
    Some(550),
    "the re-send inherits the remaining budget of the newest overtaken group: 800 - 250"
  );
}

/// And the one thing classic is: it asks for nothing.
#[test]
fn a_classic_downshift_asks_for_nothing() {
  let sets = [set_with_buffer(1, 800)];
  let mut algorithm = WasdAlgorithm::new();
  climb(&mut algorithm, &sets, 2);

  let congested = [stream(1, track_at(1, 2), 4), stream(1, track_at(1, 2), 5)];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 40));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the downshift itself is the same walk as in every version"
  );
  assert!(
    selection.supersede.is_empty(),
    "and the groups already in flight keep delivering the member they were carrying"
  );
}

/// The comparison is only worth running if the three versions walk the same:
/// same readings, same tier, version after version — they differ in one
/// request, not in the decision.
#[test]
fn the_versions_pick_the_same_tiers_from_the_same_readings() {
  let sets = [set(1, true)];
  let mut classic = WasdAlgorithm::new();
  let mut v2 = WasdAlgorithm::v2();
  let mut v3 = WasdAlgorithm::v3();

  let events = [
    (0u64, DecisionReason::GroupBoundary, Vec::new()),
    (1, DecisionReason::GroupBoundary, Vec::from(deep_at(1))),
    (2, DecisionReason::GroupBoundary, Vec::new()),
    (3, DecisionReason::GroupBoundary, Vec::new()),
    (4, DecisionReason::GroupBoundary, Vec::new()),
    (5, DecisionReason::GroupBoundary, Vec::new()),
    (6, DecisionReason::GroupBoundary, Vec::new()),
    // Mid-group samples repeat the group in flight: the new group id is
    // the boundary.
    (6, DecisionReason::PeriodicSample, Vec::from(deep_at(1))),
    (6, DecisionReason::PeriodicSample, Vec::from(deep_at(1))),
  ];
  for (group, reason, streams) in events {
    let input = input(&sets, &streams, reason, group);
    let a = classic.decide(&input);
    let b = v2.decide(&input);
    let c = v3.decide(&input);
    assert_eq!(a.tracks, b.tracks, "classic and v2 chose different tiers");
    assert_eq!(a.tracks, c.tracks, "classic and v3 chose different tiers");
  }
}

#[test]
fn an_upshift_asks_for_nothing() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::v3();
  let empty: Vec<StreamSnapshot> = Vec::new();
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 1));
  assert_eq!(tier_of(&selection, 1), 1);
  assert!(
    selection.supersede.is_empty(),
    "climbing overtakes nothing: the deliveries below the new rung are not rejected"
  );
}

/// Two streams in flight at one tier of set 1: the depth, at that rung.
fn deep_at(tier: usize) -> [StreamSnapshot; 2] {
  [
    stream(1, track_at(1, tier), 4),
    stream(1, track_at(1, tier), 5),
  ]
}

/// A connection past its exploration, with room to fall.
///
/// It climbs one rung, meets congestion at a boundary — which ends the
/// exploration and drops it to the bottom — and then sees `clean_groups`
/// uncongested boundaries, five of which buy one rung on the cautious streak.
fn settle(algorithm: &mut WasdAlgorithm, sets: &[SetSnapshot], clean_groups: u64) {
  climb(algorithm, sets, 1);
  let congested = deep_at(1);
  algorithm.decide(&input(sets, &congested, DecisionReason::GroupBoundary, 10));
  let clear: Vec<StreamSnapshot> = Vec::new();
  for group in 20..20 + clean_groups {
    algorithm.decide(&input(sets, &clear, DecisionReason::GroupBoundary, group));
  }
}

/// A healthy group's tail shows one deep reading, because the next group's
/// stream opens before the previous one has drained. The streak is what
/// separates that blip from an overload.
#[test]
fn one_deep_reading_inside_a_group_is_not_enough() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::v3();
  settle(&mut algorithm, &sets, 5);

  let inside = deep_at(1);
  let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 24));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "a single mid-group reading is the group's tail, not an overload"
  );

  let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 24));
  assert_eq!(
    tier_of(&selection, 1),
    0,
    "two of them, one after another, are"
  );
  assert!(
    selection.supersede.contains_key(&1),
    "and the groups it overtook are named, as on a boundary downshift"
  );
  assert!(
    selection.supersede[&1].age_staged,
    "and staged by age, as on a boundary downshift"
  );
}

/// The other half of the same truth: a mid-group reading that does not fire
/// must touch nothing. The handoff overlap is one deep sample right after
/// the new group's stream opens, and if it cost the climb its accumulated
/// clean streak, a connection on a healthy-but-overlapping path could never
/// gather the cautious five and would sit at the bottom rung forever.
#[test]
fn a_transient_mid_group_overlap_does_not_cost_the_climb_streak() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::v3();
  // Four clean boundaries after the fall: one short of the cautious five.
  settle(&mut algorithm, &sets, 4);

  // The handoff overlap: one deep mid-group reading, short of the streak.
  let overlap = deep_at(0);
  let selection = algorithm.decide(&input(&sets, &overlap, DecisionReason::PeriodicSample, 23));
  assert_eq!(
    tier_of(&selection, 1),
    0,
    "one reading does not fire the monitor"
  );

  // The streak it found standing is the streak it left standing: this fifth
  // clean boundary is the one the four before it paid for.
  let clear: Vec<StreamSnapshot> = Vec::new();
  let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, 24));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the transient reading did not zero the clean streak it passed through"
  );
}

/// Boundary detection is strictly monotonic: a late reading naming an
/// older group — a tail object still draining after the gate moved on — is
/// a mid-group reading, not a second boundary, and it never moves the
/// boundary clock back.
#[test]
fn an_old_group_reading_is_never_a_boundary() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::new();
  // Exploration climbs to tier 1 at group 0; a congested boundary at group
  // 10 falls back to tier 0; four clean boundaries (20..=23) and this
  // fifth (24) climb back to tier 1 with the streak reset.
  settle(&mut algorithm, &sets, 4);
  let clear: Vec<StreamSnapshot> = Vec::new();
  let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, 24));
  assert_eq!(tier_of(&selection, 1), 1, "the fifth clean boundary climbs");

  // A tail object of the old group 4, deep at the delivered tier, while
  // the newest group decided was 24. Taken for a boundary this is a
  // congested one — a downshift. It is a mid-group reading, one short of
  // the monitor's streak, and nothing fires.
  let overlap = deep_at(1);
  let selection = algorithm.decide(&input(&sets, &overlap, DecisionReason::PeriodicSample, 4));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "an older group's reading cannot open a congested boundary"
  );

  // And the clock did not move back to 4: a clean reading for the newest
  // group is still mid-group, so it is not a fifth-plus-one clean
  // boundary either.
  let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::PeriodicSample, 24));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the newest group is still judged against the newest group"
  );

  // The next climb waits the full cautious five from where the upshift
  // left it: the old reading and the mid-group clean one are not clean
  // boundaries, so four are not enough...
  for group in 25..29 {
    let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, group));
    assert_eq!(
      tier_of(&selection, 1),
      1,
      "tails do not count as clean groups"
    );
  }
  // ...and the fifth is.
  let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, 29));
  assert_eq!(
    tier_of(&selection, 1),
    2,
    "the clock never moved back to the draining tail"
  );
}

#[test]
fn the_monitor_rearms_after_it_fires() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::new();
  settle(&mut algorithm, &sets, 10);

  // Two readings at the tier being served, and the monitor steps down.
  // Mid-group samples repeat the group in flight: the new group id is the
  // boundary.
  let at_two = deep_at(2);
  algorithm.decide(&input(&sets, &at_two, DecisionReason::PeriodicSample, 29));
  let selection = algorithm.decide(&input(&sets, &at_two, DecisionReason::PeriodicSample, 29));
  assert_eq!(tier_of(&selection, 1), 1, "the monitor fired");

  // The step drained the signal it was reading: those same streams are at a
  // rung the algorithm has now rejected. One fresh reading at the new tier must
  // not be enough, because a streak left standing at the threshold would make it
  // so, and the ladder would walk itself down tick by tick.
  let at_one = deep_at(1);
  let selection = algorithm.decide(&input(&sets, &at_one, DecisionReason::PeriodicSample, 29));
  assert_eq!(tier_of(&selection, 1), 1, "the streak re-armed");

  algorithm.decide(&input(&sets, &at_one, DecisionReason::PeriodicSample, 29));
  let selection = algorithm.decide(&input(&sets, &at_one, DecisionReason::PeriodicSample, 29));
  assert_eq!(
    tier_of(&selection, 1),
    0,
    "a full second streak costs a full second streak"
  );
}

/// The exploration walks the ladder one group at a time, and a reading taken
/// inside a group is not a group: the monitor never fires mid-exploration.
/// The boundary is where the exploration learns the ladder is taller than the
/// path, and it needs only its one reading to say so.
#[test]
fn the_monitor_never_interrupts_the_exploration() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::v3();
  climb(&mut algorithm, &sets, 1);

  let inside = [stream(1, track_at(1, 1), 9), stream(1, track_at(1, 1), 10)];
  for _ in 0..5 {
    let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 0));
    assert_eq!(
      tier_of(&selection, 1),
      1,
      "the exploration is walked in whole groups"
    );
    assert!(selection.supersede.is_empty());
  }

  // The boundary reading is where the exploration learns the ladder is taller
  // than the path, and from there the monitor is armed.
  let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::GroupBoundary, 11));
  assert_eq!(tier_of(&selection, 1), 0);
  assert!(selection.supersede.contains_key(&1));
}

/// A stream the transport gave up on is a consequence of a decision the
/// algorithm already made. Acting on it as well would count the same event
/// twice, and the depth has already changed by the time anyone asks.
#[test]
fn a_stream_timeout_is_not_a_reading() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::v3();
  climb(&mut algorithm, &sets, 2);

  let timeouts = [(1u64, 3u64)].into_iter().collect::<HashMap<u64, u64>>();
  let empty: Vec<StreamSnapshot> = Vec::new();
  let input = AbrInput {
    client_id: 1,
    group_id: 40,
    bandwidth_estimate_kbps: 0,
    configured_cap_kbps: 0,
    sets: &sets,
    reason: DecisionReason::StreamTimeout,
    now_ms: 0,
    streams: &empty,
    stream_timeouts_since_last_decision: &timeouts,
  };
  let selection = algorithm.decide(&input);
  assert_eq!(
    tier_of(&selection, 1),
    2,
    "the reset is not evidence of a tier that is too high"
  );
  assert!(selection.supersede.is_empty());
}

/// The exploration is over for good at the top of the ladder: the cautious
/// streak is what the rest of the connection climbs on, and one clean group is
/// not enough again.
#[test]
fn the_cautious_streak_returns_after_the_exploration_is_over() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::new();
  // Four clean groups reach the top of a four-rung ladder and end the
  // exploration there.
  let top = climb(&mut algorithm, &sets, 4);
  assert_eq!(top, 3);

  // Fall one rung: a congested boundary reading is certain on its own.
  let congested = [stream(1, track_at(1, 3), 20), stream(1, track_at(1, 3), 21)];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 30));
  assert_eq!(tier_of(&selection, 1), 2);

  // One clean group does not climb back: the second climb is the cautious one.
  let clear: Vec<StreamSnapshot> = Vec::new();
  let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, 31));
  assert_eq!(tier_of(&selection, 1), 2);

  // Five are.
  for group in 32..36 {
    algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, group));
  }
  let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, 36));
  assert_eq!(tier_of(&selection, 1), 3);
}

/// U_min = ceil(B / G) is the floor of the cautious streak: the testbed's
/// 1600 ms buffer over a 500 ms cadence gives 4, and at rho = 1.0 it
/// dominates every step of a ladder whose relative bitrate jumps are all
/// smaller than one.
#[test]
fn u_min_scales_with_subscriber_buffer_and_group_span() {
  let sets = [ladder_set(1, &[850, 1800, 3500, 6000], 1600, 500)];
  let mut algorithm = WasdAlgorithm::new();
  let empty: Vec<StreamSnapshot> = Vec::new();

  // One clean group climbs to tier 1 while exploring; a congested boundary
  // there ends the exploration and falls back to tier 0.
  algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 0));
  let congested = [stream(1, track_at(1, 1), 1), stream(1, track_at(1, 1), 2)];
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 1));

  // U(0) = max(4, ceil((1800 - 850) / 850 / 1.0)) = max(4, ceil(1.118)) =
  // max(4, 2) = 4: the floor governs, so three clean groups are not enough.
  for group in 2..5 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(tier_of(&selection, 1), 0, "U_min = 4, not fewer");
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 5));
  assert_eq!(tier_of(&selection, 1), 1, "the fourth clean group climbs");

  // U(1) = max(4, ceil(0.944 / 1.0)) = 4 as well: the floor, not the step.
  for group in 6..9 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(tier_of(&selection, 1), 1);
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 9));
  assert_eq!(
    tier_of(&selection, 1),
    2,
    "and the next step waits four too"
  );
}

/// A smaller rho pays for its caution in groups: the step 850 -> 1800 is a
/// 112% jump, and at rho = 0.15 it costs ceil(1.1176 / 0.15) = 8 clean
/// groups - the step term ruling above the U_min floor of 4.
#[test]
fn smaller_rho_increases_upshift_streak_for_large_bitrate_jumps() {
  let sets = [ladder_set(1, &[850, 1800, 3500, 6000], 1600, 500)];
  let mut algorithm = WasdAlgorithm::with_params(WasdParams {
    monitor_reads: 2,
    rho: 0.15,
    ..WasdParams::default()
  });
  let empty: Vec<StreamSnapshot> = Vec::new();

  // Congestion at the bottom tier ends the exploration without a fall.
  let congested = [stream(1, track_at(1, 0), 0), stream(1, track_at(1, 0), 1)];
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));

  for group in 1..8 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(
      tier_of(&selection, 1),
      0,
      "rho = 0.15 prices this step at 8"
    );
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 8));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the eighth clean group buys the jump"
  );
}

/// The subscriber's own signalled tolerance overrides the operator's default
/// for its own set: a trailing `algorithm_params` percentage (`1..=100`)
/// becomes rho. Here the algorithm is built at the default rho = 1.0, but the
/// set carries `15`, so the 850 -> 1800 jump is priced at rho = 0.15 - eight
/// clean groups - not the four the default would allow.
#[test]
fn a_signalled_algorithm_param_overrides_rho() {
  let sets = [SetSnapshot {
    algorithm_params: vec![15],
    ..ladder_set(1, &[850, 1800, 3500, 6000], 1600, 500)
  }];
  let mut algorithm = WasdAlgorithm::new();
  let empty: Vec<StreamSnapshot> = Vec::new();

  // Congestion at the bottom tier ends the exploration without a fall.
  let congested = [stream(1, track_at(1, 0), 0), stream(1, track_at(1, 0), 1)];
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));

  // At the default rho = 1.0 this step would cost max(4, ceil(1.118)) = 4; the
  // signalled 15 drives it to rho = 0.15, so seven clean groups are not enough.
  for group in 1..8 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(
      tier_of(&selection, 1),
      0,
      "the signalled rho = 0.15 prices this step at 8, not the default 4"
    );
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 8));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the eighth clean group buys the jump"
  );
}

/// A subscriber that sends the two-slot layout signals `U_min` in slot 0, in
/// units of media groups: the signalled 6 rules over the derived
/// `ceil(1600 / 500) = 4` floor, so the climb waits six clean groups where
/// the clock alone would have allowed four. (The step's own price, at the
/// signalled rho = 100 % giving ceil(1.118 / 1.0) = 2, stays under both.)
#[test]
fn a_signalled_u_min_overrides_the_buffer_floor() {
  let sets = [SetSnapshot {
    algorithm_params: vec![6, 100],
    ..ladder_set(1, &[850, 1800, 3500, 6000], 1600, 500)
  }];
  let mut algorithm = WasdAlgorithm::new();
  let empty: Vec<StreamSnapshot> = Vec::new();

  // Congestion at the bottom tier ends the exploration without a fall.
  let congested = [stream(1, track_at(1, 0), 0), stream(1, track_at(1, 0), 1)];
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));

  for group in 1..6 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(
      tier_of(&selection, 1),
      0,
      "the signalled U_min = 6, not the derived 4"
    );
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 6));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the sixth clean group buys the jump"
  );
}

/// A zero in the `U_min` slot means "not signalled": the derived floor
/// `ceil(B / G) = 4` stands. And when no one signalled and a clock is
/// unknown - here a set whose first group has not closed, so no cadence -
/// the written default of five governs.
#[test]
fn an_unsignalled_u_min_falls_back_to_the_clock_then_the_default() {
  let empty: Vec<StreamSnapshot> = Vec::new();
  let congested = [stream(1, track_at(1, 0), 0), stream(1, track_at(1, 0), 1)];

  // Slot 0 zero: not signalled, so the clock answers: max(4, 2) = 4.
  let sets = [SetSnapshot {
    algorithm_params: vec![0, 100],
    ..ladder_set(1, &[850, 1800, 3500, 6000], 1600, 500)
  }];
  let mut algorithm = WasdAlgorithm::new();
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));
  for group in 1..4 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(tier_of(&selection, 1), 0, "the derived 4, not fewer");
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 4));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "ceil(1600 / 500) = 4 buys the jump"
  );

  // No cadence and no signal: the written UPSHIFT_GOP_STREAK of five.
  let sets = [SetSnapshot {
    algorithm_params: vec![0, 100],
    ..ladder_set(1, &[850, 1800, 3500, 6000], 1600, 0)
  }];
  let mut algorithm = WasdAlgorithm::new();
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));
  for group in 1..5 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(tier_of(&selection, 1), 0, "the written 5, not fewer");
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 5));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the unknown clock falls back to UPSHIFT_GOP_STREAK"
  );
}

/// With both slots signalled, slot 0 is the floor and the trailing one is
/// the tolerance, and `U(k)` is the max of the floor and the step's price:
/// at rho = 0.15 the 850 -> 1800 jump costs ceil(1.118 / 0.15) = 8, so
/// `vec![10, 15]` waits the signalled 10 and `vec![3, 15]` waits the step's 8.
#[test]
fn u_min_and_rho_signalled_together_take_the_max() {
  let empty: Vec<StreamSnapshot> = Vec::new();
  let congested = [stream(1, track_at(1, 0), 0), stream(1, track_at(1, 0), 1)];

  // The floor rules the step: max(10, 8) = 10.
  let sets = [SetSnapshot {
    algorithm_params: vec![10, 15],
    ..ladder_set(1, &[850, 1800, 3500, 6000], 1600, 500)
  }];
  let mut algorithm = WasdAlgorithm::new();
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));
  for group in 1..10 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(
      tier_of(&selection, 1),
      0,
      "the signalled floor of 10 rules the step's 8"
    );
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 10));
  assert_eq!(tier_of(&selection, 1), 1, "the tenth clean group buys it");

  // The step rules the floor: max(3, 8) = 8.
  let sets = [SetSnapshot {
    algorithm_params: vec![3, 15],
    ..ladder_set(1, &[850, 1800, 3500, 6000], 1600, 500)
  }];
  let mut algorithm = WasdAlgorithm::new();
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));
  for group in 1..8 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(
      tier_of(&selection, 1),
      0,
      "the step's 8 rules the signalled 3"
    );
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 8));
  assert_eq!(tier_of(&selection, 1), 1, "the eighth clean group buys it");
}

/// The operator's `wasd.umin` parses through the tuning: zero is unset and a
/// positive count is the floor.
#[test]
fn the_operator_tuning_parses_the_u_min_floor() {
  let tuned = crate::Tuning::from_pairs(["wasd.umin=6"]);
  assert_eq!(WasdParams::from_tuning(&tuned).u_min, Some(6));

  let unset = crate::Tuning::from_pairs(["wasd.umin=0"]);
  assert_eq!(WasdParams::from_tuning(&unset).u_min, None);

  let untouched = crate::Tuning::from_pairs(std::iter::empty::<&str>());
  assert_eq!(
    WasdParams::from_tuning(&untouched).u_min,
    None,
    "an untouched key keeps the derived horizon"
  );
}

/// The operator's `wasd.umin` replaces the derived horizon for a subscriber
/// that signals none — and loses to one that does: the same ladder climbs
/// after six clean groups under the operator's floor of 6, and after three
/// under the signalled `vec![3, 100]`.
#[test]
fn the_operator_u_min_replaces_the_clock_but_loses_to_the_subscriber() {
  let empty: Vec<StreamSnapshot> = Vec::new();
  let congested = [stream(1, track_at(1, 0), 0), stream(1, track_at(1, 0), 1)];

  // Operator floor 6, subscriber silent: max(6, 2) = 6, not the derived 4.
  let sets = [ladder_set(1, &[850, 1800, 3500, 6000], 1600, 500)];
  let mut algorithm = WasdAlgorithm::with_params(WasdParams {
    u_min: Some(6),
    ..WasdParams::default()
  });
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));
  for group in 1..6 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(
      tier_of(&selection, 1),
      0,
      "the operator's floor of 6, not the derived 4"
    );
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 6));
  assert_eq!(tier_of(&selection, 1), 1, "the sixth clean group buys it");

  // The subscriber's signalled 3 outranks the operator's 6: max(3, 2) = 3.
  let sets = [SetSnapshot {
    algorithm_params: vec![3, 100],
    ..ladder_set(1, &[850, 1800, 3500, 6000], 1600, 500)
  }];
  let mut algorithm = WasdAlgorithm::with_params(WasdParams {
    u_min: Some(6),
    ..WasdParams::default()
  });
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));
  for group in 1..3 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(
      tier_of(&selection, 1),
      0,
      "the signalled 3 outranks the operator's 6"
    );
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 3));
  assert_eq!(tier_of(&selection, 1), 1, "the third clean group buys it");
}

/// The same connection pays per step, not per ladder: a 3x jump costs eight
/// clean groups at rho = 0.25 while the 1.2x step right behind it costs only
/// the U_min floor of two.
#[test]
fn non_uniform_ladder_requires_more_groups_for_larger_relative_jumps() {
  let sets = [ladder_set(1, &[500, 1500, 1800], 1000, 500)];
  let mut algorithm = WasdAlgorithm::with_params(WasdParams {
    monitor_reads: 2,
    rho: 0.25,
    ..WasdParams::default()
  });
  let empty: Vec<StreamSnapshot> = Vec::new();

  let congested = [stream(1, track_at(1, 0), 0), stream(1, track_at(1, 0), 1)];
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 0));

  // U(0) = max(2, ceil(2.0 / 0.25)) = 8 for the 500 -> 1500 step.
  for group in 1..8 {
    let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, group));
    assert_eq!(tier_of(&selection, 1), 0);
  }
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 8));
  assert_eq!(tier_of(&selection, 1), 1, "the 3x jump costs eight");

  // U(1) = max(2, ceil(0.8)) = 2 for the 1500 -> 1800 step.
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 9));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the small step starts on the floor"
  );
  let selection = algorithm.decide(&input(&sets, &empty, DecisionReason::GroupBoundary, 10));
  assert_eq!(tier_of(&selection, 1), 2, "and takes only two");
}

/// A re-sent group inherits the remaining buffer budget of the overtaken
/// stream it replaces. The overtaken stream has already spent part of the
/// subscriber's window on the wire; what it has left is exactly what its
/// replacement has left, so the newest overtaken group donates its remainder.
#[test]
fn a_downshift_inherits_the_remaining_buffer_of_the_newest_overtaken_stream() {
  let sets = [set_with_buffer(1, 800)];
  let mut algorithm = WasdAlgorithm::v3();
  climb(&mut algorithm, &sets, 2);

  let congested = [
    stream_with_age(1, track_at(1, 2), 7, 500),
    stream_with_age(1, track_at(1, 2), 8, 300),
  ];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 20));
  assert_eq!(tier_of(&selection, 1), 1, "the tier fell");
  assert_eq!(
    selection.resend_deadline_ms(1),
    Some(500),
    "the re-send's deadline is the newest overtaken group's remainder: 800 - 300"
  );
  assert!(
    selection.supersede[&1].age_staged,
    "and the staging does not depend on the deadline"
  );
}

/// A stream that is itself a re-send donates a deadline too: its age runs
/// from the moment the repair started, which is exactly what a second
/// repair of the same group has left of the subscriber's window. The depth
/// exclusion does not apply here - the depth asks what the network is
/// doing, the deadline asks what the delivery has left - so the filter
/// that keeps the re-send out of the depth must not reach this one.
#[test]
fn a_re_sent_stream_donates_its_remaining_buffer_to_the_next_repair() {
  let sets = [set_with_buffer(1, 800)];
  let mut algorithm = WasdAlgorithm::v3();
  climb(&mut algorithm, &sets, 2);

  // Two fresh streams at tier 2 make the depth that steps the ladder. The
  // newest stream above the new rung is then a re-send from an earlier
  // step, 200 ms into its delivery: newest group, so it donates - the
  // depth exclusion must not reach here.
  let congested = [
    stream_with_age(1, track_at(1, 2), 7, 500),
    stream_with_age(1, track_at(1, 2), 8, 100),
    StreamSnapshot {
      age_ms: 200,
      ..resent(1, track_at(1, 2), 9)
    },
  ];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 20));
  assert_eq!(tier_of(&selection, 1), 1, "the tier fell");
  assert_eq!(
    selection.resend_deadline_ms(1),
    Some(600),
    "the newest overtaken group is the re-send: 800 - 200, not the fresh 700"
  );
}

/// A set that negotiated no buffer granted no permission to discard its media,
/// and there is nothing to halve. Asking for no deadline leaves the relay's rule
/// of never discarding in charge; inventing a short one would spend a budget the
/// subscriber never offered, and 0 in this contract means never, not immediately.
#[test]
fn a_set_that_negotiated_no_buffer_asks_for_no_deadline() {
  let sets = [set_with_buffer(1, 0)];
  let mut algorithm = WasdAlgorithm::v3();
  climb(&mut algorithm, &sets, 2);

  let congested = [stream(1, track_at(1, 2), 7), stream(1, track_at(1, 2), 8)];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 20));
  assert!(
    selection.supersede.contains_key(&1),
    "the overtaken groups are still named"
  );
  assert_eq!(
    selection.resend_deadline_ms(1),
    None,
    "and nothing is asked of their deadlines"
  );
  assert!(
    selection.supersede[&1].age_staged,
    "the staging stands on its own: no buffer, no deadline, but the older groups still go one rung below"
  );
}

/// An overtaken stream whose age already meets or exceeds the buffer has no
/// remaining budget to donate, and the re-send must expire immediately rather
/// than fall through to 0 — 0 means "never" in this contract, so running out
/// of arithmetic room must not turn an expired deadline into an infinite one.
#[test]
fn exhausted_remaining_buffer_clamps_to_one_millisecond_not_never() {
  let sets = [set_with_buffer(1, 800)];
  let mut algorithm = WasdAlgorithm::v3();
  climb(&mut algorithm, &sets, 2);

  let congested = [
    stream_with_age(1, track_at(1, 2), 7, 850),
    stream_with_age(1, track_at(1, 2), 8, 850),
  ];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 20));
  assert_eq!(selection.resend_deadline_ms(1), Some(1));
}

/// The unified path prices the whole group: an upshift recorded at the start
/// of group N and the congestion found at the end of that same group are two
/// readings of one clock. The downshift acts on the group in flight — the
/// supersede names it — and the climb starts over from zero, so the next
/// group does not bounce straight back up.
#[test]
fn a_downshift_inside_a_group_undoes_the_upshift_recorded_at_its_start() {
  let sets = [set_with_buffer(1, 800)];
  let mut algorithm = WasdAlgorithm::v2();
  settle(&mut algorithm, &sets, 5);

  // The cautious streak buys one rung: group 34 opens clean and is served one
  // tier up.
  let clear: Vec<StreamSnapshot> = Vec::new();
  let mut selection = None;
  for group in 30..35 {
    selection = Some(algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, group)));
  }
  assert_eq!(
    tier_of(&selection.unwrap(), 1),
    2,
    "group 34 was recorded one rung up"
  );

  // Congestion at the end of the same group: the monitor's two readings, and
  // the downshift lands on group 34 itself.
  let inside = deep_at(2);
  algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 34));
  let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 34));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "the tier falls back down inside the group it was raised in"
  );
  assert!(
    selection.supersede.contains_key(&1) && selection.supersede[&1].newest_only,
    "and the upshifted group in flight is overtaken like any other"
  );

  // The dip has to earn a full fresh clean streak before the ladder climbs
  // back: the next group stays at the downshifted tier.
  let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, 35));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "clear_streak was zeroed by the congestion: one clean group does not undo the fall"
  );
}
