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
  // falls once the monitor's streak is full: one reading is a tail, two are a
  // verdict, at a boundary or anywhere else.
  let here = [stream(1, track_at(1, 3), 13), stream(1, track_at(1, 3), 14)];
  algorithm.decide(&input(&sets, &here, DecisionReason::GroupBoundary, 21));
  let selection = algorithm.decide(&input(&sets, &here, DecisionReason::GroupBoundary, 22));
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
  algorithm.decide(&input(&sets, &fresh, DecisionReason::GroupBoundary, 31));
  let selection = algorithm.decide(&input(&sets, &fresh, DecisionReason::GroupBoundary, 32));
  assert_eq!(
    tier_of(&selection, 1),
    2,
    "the same two streams, at the tier now being served, are"
  );
}

#[test]
fn a_v3_downshift_asks_to_overtake_the_sets_it_moved_down() {
  let sets = [set(1, true), set(2, false)];
  let mut algorithm = WasdAlgorithm::v3();
  climb(&mut algorithm, &sets, 2);

  let congested = [stream(1, track_at(1, 2), 4), stream(1, track_at(1, 2), 5)];
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 40));
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 41));
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
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 40));
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 41));
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
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 40));
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 41));
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
    (DecisionReason::GroupBoundary, Vec::new()),
    (DecisionReason::GroupBoundary, Vec::from(deep_at(1))),
    (DecisionReason::GroupBoundary, Vec::new()),
    (DecisionReason::GroupBoundary, Vec::new()),
    (DecisionReason::GroupBoundary, Vec::new()),
    (DecisionReason::GroupBoundary, Vec::new()),
    (DecisionReason::GroupBoundary, Vec::new()),
    (DecisionReason::PeriodicSample, Vec::from(deep_at(1))),
    (DecisionReason::PeriodicSample, Vec::from(deep_at(1))),
  ];
  for (group, (reason, streams)) in events.iter().enumerate() {
    let input = input(&sets, streams, *reason, group as u64);
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
/// It climbs one rung, meets congestion at that rung — two readings, because
/// every downshift is the monitor's now, and the second ends the exploration
/// and drops it to the bottom — and then sees `clean_groups` uncongested
/// boundaries, five of which buy one rung on the cautious streak.
fn settle(algorithm: &mut WasdAlgorithm, sets: &[SetSnapshot], clean_groups: u64) {
  climb(algorithm, sets, 1);
  let congested = deep_at(1);
  algorithm.decide(&input(sets, &congested, DecisionReason::GroupBoundary, 10));
  algorithm.decide(&input(sets, &congested, DecisionReason::GroupBoundary, 11));
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
  let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 30));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "a single mid-group reading is the group's tail, not an overload"
  );

  let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 30));
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

#[test]
fn the_monitor_rearms_after_it_fires() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::new();
  settle(&mut algorithm, &sets, 10);

  // Two readings at the tier being served, and the monitor steps down.
  let at_two = deep_at(2);
  algorithm.decide(&input(&sets, &at_two, DecisionReason::PeriodicSample, 30));
  let selection = algorithm.decide(&input(&sets, &at_two, DecisionReason::PeriodicSample, 30));
  assert_eq!(tier_of(&selection, 1), 1, "the monitor fired");

  // The step drained the signal it was reading: those same streams are at a
  // rung the algorithm has now rejected. One fresh reading at the new tier must
  // not be enough, because a streak left standing at the threshold would make it
  // so, and the ladder would walk itself down tick by tick.
  let at_one = deep_at(1);
  let selection = algorithm.decide(&input(&sets, &at_one, DecisionReason::PeriodicSample, 31));
  assert_eq!(tier_of(&selection, 1), 1, "the streak re-armed");

  algorithm.decide(&input(&sets, &at_one, DecisionReason::PeriodicSample, 31));
  let selection = algorithm.decide(&input(&sets, &at_one, DecisionReason::PeriodicSample, 31));
  assert_eq!(
    tier_of(&selection, 1),
    0,
    "a full second streak costs a full second streak"
  );
}

/// The unified monitor: one persistence counter for every sample, so the
/// exploration is no longer shielded from mid-group readings. Two consecutive
/// congested samples end the exploration and step down, boundary or not.
#[test]
fn the_monitor_ends_the_exploration_when_it_fires() {
  let sets = [set(1, true)];
  let mut algorithm = WasdAlgorithm::v3();
  climb(&mut algorithm, &sets, 1);

  let inside = [stream(1, track_at(1, 1), 9), stream(1, track_at(1, 1), 10)];
  let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 9));
  assert_eq!(
    tier_of(&selection, 1),
    1,
    "one reading is not yet the monitor's, exploring or not"
  );
  assert!(selection.supersede.is_empty());

  let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 9));
  assert_eq!(
    tier_of(&selection, 1),
    0,
    "the second ends the exploration and steps down: sustained congestion is
     narrower than the ladder wherever the sample lands"
  );
  assert!(
    selection.supersede.contains_key(&1),
    "and the downshift reaches back like any other"
  );
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

  // Fall one rung: the monitor's two readings, wherever they land.
  let congested = [stream(1, track_at(1, 3), 20), stream(1, track_at(1, 3), 21)];
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 30));
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 31));
  assert_eq!(tier_of(&selection, 1), 2);

  // One clean group does not climb back: the second climb is the cautious one.
  let clear: Vec<StreamSnapshot> = Vec::new();
  let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, 32));
  assert_eq!(tier_of(&selection, 1), 2);

  // Five are.
  for group in 33..36 {
    algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, group));
  }
  let selection = algorithm.decide(&input(&sets, &clear, DecisionReason::GroupBoundary, 36));
  assert_eq!(tier_of(&selection, 1), 3);
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
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 20));
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 21));
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
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 20));
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 21));
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
  algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 20));
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 21));
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
