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

//! Tests for the three things eeasd changes: where the depth is measured, when
//! it is read, and what a downshift asks the host to do about the groups it
//! overtook.

use std::collections::HashMap;

use crate::AbrAlgorithm;
use crate::AbrInput;
use crate::DecisionReason;
use crate::SetSnapshot;
use crate::StreamSnapshot;
use crate::algorithms::eeasd::EeasdAlgorithm;

/// A set with a four-rung ladder. The rungs are addressed by index, so tier
/// `n` of set `id` is track `id * 10 + n`.
fn set(id: u64, active: bool) -> SetSnapshot {
  SetSnapshot {
    id,
    algorithm_id: super::EEASD_ALGORITHM_ID,
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

/// The same set with the subscriber's reorder buffer filled in, which is what
/// eeasd halves for a group it causes to be re-sent.
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
fn climb(algorithm: &mut EeasdAlgorithm, sets: &[SetSnapshot], groups: u64) -> usize {
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
  let mut algorithm = EeasdAlgorithm::new();

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
  let mut algorithm = EeasdAlgorithm::new();
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

  // The same count at the tier being delivered is congestion, and the tier falls.
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
  let mut algorithm = EeasdAlgorithm::new();
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

#[test]
fn a_downshift_asks_to_overtake_the_sets_it_moved_down() {
  let sets = [set(1, true), set(2, false)];
  let mut algorithm = EeasdAlgorithm::new();
  climb(&mut algorithm, &sets, 2);

  let congested = [stream(1, track_at(1, 2), 4), stream(1, track_at(1, 2), 5)];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 40));
  assert!(
    selection.supersede.contains_key(&1),
    "the groups still in flight above the new rung were overtaken"
  );
  assert!(
    !selection.supersede.contains_key(&2),
    "a set with no subscriber has no delivery to overtake"
  );
}

#[test]
fn an_upshift_asks_for_nothing() {
  let sets = [set(1, true)];
  let mut algorithm = EeasdAlgorithm::new();
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
/// It climbs one rung, meets congestion at that rung — which is what ends the
/// exploration and drops it to the bottom — and then sees `clean_groups`
/// uncongested boundaries, five of which buy one rung on the cautious streak.
fn settle(algorithm: &mut EeasdAlgorithm, sets: &[SetSnapshot], clean_groups: u64) {
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
  let mut algorithm = EeasdAlgorithm::new();
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
}

#[test]
fn the_monitor_rearms_after_it_fires() {
  let sets = [set(1, true)];
  let mut algorithm = EeasdAlgorithm::new();
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

/// The exploration walks the ladder one group at a time, and a reading taken
/// inside a group is not a group. Mid-transmission downshifts begin once the
/// connection has seen the ladder's height.
#[test]
fn the_monitor_never_interrupts_the_exploration() {
  let sets = [set(1, true)];
  let mut algorithm = EeasdAlgorithm::new();
  climb(&mut algorithm, &sets, 1);

  let inside = [stream(1, track_at(1, 1), 9), stream(1, track_at(1, 1), 10)];
  for _ in 0..5 {
    let selection = algorithm.decide(&input(&sets, &inside, DecisionReason::PeriodicSample, 9));
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
  let mut algorithm = EeasdAlgorithm::new();
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
  let mut algorithm = EeasdAlgorithm::new();
  // Four clean groups reach the top of a four-rung ladder and end the
  // exploration there.
  let top = climb(&mut algorithm, &sets, 4);
  assert_eq!(top, 3);

  // Fall one rung.
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

/// A re-sent group gets half the buffer the subscriber negotiated for the set.
/// It starts behind the delivery it replaced, so the deadline it is measured
/// against is the one it can still make, and choosing this algorithm is how the
/// subscriber agreed to it.
#[test]
fn a_downshift_asks_for_half_the_negotiated_buffer_on_the_re_send() {
  let sets = [set_with_buffer(1, 800)];
  let mut algorithm = EeasdAlgorithm::new();
  climb(&mut algorithm, &sets, 2);

  let congested = [stream(1, track_at(1, 2), 7), stream(1, track_at(1, 2), 8)];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 20));
  assert_eq!(tier_of(&selection, 1), 1, "the tier fell");
  assert_eq!(
    selection.resend_deadline_ms(1),
    Some(400),
    "the re-send is to be done with by half the buffer"
  );
}

/// A set that negotiated no buffer granted no permission to discard its media,
/// and there is nothing to halve. Asking for no deadline leaves the relay's rule
/// of never discarding in charge; inventing a short one would spend a budget the
/// subscriber never offered, and 0 in this contract means never, not immediately.
#[test]
fn a_set_that_negotiated_no_buffer_asks_for_no_deadline() {
  let sets = [set_with_buffer(1, 0)];
  let mut algorithm = EeasdAlgorithm::new();
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
}

/// Half of 1 ms is 0, and 0 means never. A buffer that short is a rounding
/// error rather than a policy, but the deadline must not become infinite
/// because the arithmetic ran out of room.
#[test]
fn halving_a_buffer_of_one_millisecond_does_not_mean_never() {
  let sets = [set_with_buffer(1, 1)];
  let mut algorithm = EeasdAlgorithm::new();
  climb(&mut algorithm, &sets, 2);

  let congested = [stream(1, track_at(1, 2), 7), stream(1, track_at(1, 2), 8)];
  let selection = algorithm.decide(&input(&sets, &congested, DecisionReason::GroupBoundary, 20));
  assert_eq!(selection.resend_deadline_ms(1), Some(1));
}
