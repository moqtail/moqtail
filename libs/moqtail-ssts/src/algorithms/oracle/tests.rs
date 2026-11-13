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

//! Tests for the oracle's predestined walk.

use std::collections::HashMap;

use super::*;
use crate::{AbrInput, DecisionReason, SetSnapshot, Tuning};

/// An empty timeout map with a life long enough to hand to an `AbrInput`.
fn no_timeouts() -> &'static HashMap<u64, u64> {
  static NONE: std::sync::OnceLock<HashMap<u64, u64>> = std::sync::OnceLock::new();
  NONE.get_or_init(HashMap::new)
}

/// One ladder, six rungs at the nominal rates, thresholds as track ids.
fn set(id: u64, thresholds: &[u64]) -> SetSnapshot {
  SetSnapshot {
    id,
    algorithm_id: ORACLE_ALGORITHM_ID,
    rank: 0,
    weight: 1,
    active: true,
    members: thresholds.iter().map(|&t| (t, t)).collect(),
    subscriber_buffer_ms: 1600,
    algorithm_params: vec![],
    group_span_ms: 500,
  }
}

fn input<'a>(group_id: u64, now_ms: u64, sets: &'a [SetSnapshot]) -> AbrInput<'a> {
  AbrInput {
    client_id: 1,
    group_id,
    bandwidth_estimate_kbps: 0,
    configured_cap_kbps: 0,
    reason: DecisionReason::GroupBoundary,
    now_ms,
    now_unix_ms: now_ms,
    sets,
    streams: &[],
    stream_timeouts_since_last_decision: no_timeouts(),
  }
}

fn tuned(steps: &str) -> OracleAlgorithm {
  let steps = format!("oracle.steps={steps}");
  OracleAlgorithm::from_tuning(&Tuning::from_pairs([&steps, "oracle.group_ms=500"]))
}

/// The same, with the trace's t0 on the host's clock: a decision at `now_ms`
/// reads the window `[now_ms - t0_ms, now_ms - t0_ms + 500)`.
fn tuned_at(steps: &str, t0_ms: u64) -> OracleAlgorithm {
  let steps = format!("oracle.steps={steps}");
  let t0 = format!("oracle.t0_ms={t0_ms}");
  OracleAlgorithm::from_tuning(&Tuning::from_pairs([&steps, "oracle.group_ms=500", &t0]))
}

/// The window at `now_ms` is the 500 ms of the trace that starts now, and the
/// rung is the highest one whose nominal rate the window's MEAN covers: a
/// step's own window steps onto the new rung, a burst does not upshift it,
/// and a dip does not downshift it.
#[test]
fn the_window_mean_prices_the_rung() {
  // 1 Gbps for 2 s, then 3 Gbps for 2 s; the trace begins at now 0. The
  // window at 1.5 s averages 1 Gbps, at 2.0 s and 2.5 s it is the step's own,
  // and at 5.0 s it is past the schedule's end and holds the last capacity.
  let sets = [set(1, &[1000, 2000, 3000])];
  let mut oracle = tuned("1000:2000;3000:2000");

  let mut rung = |now_ms: u64| oracle.decide(&input(0, now_ms, &sets))[&1];
  assert_eq!(rung(0), Some(1000), "the trace's first window");
  assert_eq!(rung(1500), Some(1000), "1.5 s averages 1 Gbps");
  assert_eq!(rung(2000), Some(3000), "2.0 s is the step's own");
  assert_eq!(rung(2500), Some(3000), "2.5 s is the step's own too");
  assert_eq!(rung(5000), Some(3000), "5.0 s holds the last capacity");
}

/// The trace's t0 is where the schedule's clock begins: a decision at
/// `now_ms = t0 + 1.5 s` reads 1.5 s of the trace, whatever the host's clock
/// reads — the offset is the whole of the alignment.
#[test]
fn the_t0_offset_aligns_the_trace_clock() {
  let sets = [set(1, &[1000, 2000, 3000])];
  // The trace begins 3500 ms into the host's clock: a decision at 5000 ms is
  // 1.5 s of the trace, and reads the 1 Gbps step.
  let mut oracle = tuned_at("1000:2000;3000:2000", 3500);
  assert_eq!(
    oracle.decide(&input(0, 5000, &sets))[&1],
    Some(1000),
    "5000 - 3500 = 1.5 s of the trace"
  );
  // The same trace 500 ms later: 2.0 s, the 3 Gbps step's own window.
  assert_eq!(
    oracle.decide(&input(0, 5500, &sets))[&1],
    Some(3000),
    "5500 - 3500 = 2.0 s of the trace"
  );
}

/// 3 Gbps for the window's first quarter, 1 Gbps for the rest: the dip is more
/// than paid for, and the MEAN still covers the middle rung. The mirror trace
/// — 1 Gbps then 3 Gbps — averages the same 2000 and lands on the same rung:
/// the rung is the mean's, whichever way the window's halves lean.
#[test]
fn a_dip_inside_the_window_does_not_downshift_it() {
  let sets = [set(1, &[1000, 2000, 3000])];
  let mut oracle = tuned("3000:250;1000:1750");

  // The anchor: the first decision, at now 0; its window covers both halves,
  // and the mean lands on the middle rung.
  let selection = oracle.decide(&input(0, 0, &sets));
  assert_eq!(selection[&1], Some(2000));

  let mut mirror = tuned("1000:250;3000:1750");
  let selection = mirror.decide(&input(0, 0, &sets));
  assert_eq!(
    selection[&1],
    Some(2000),
    "the mirrored burst does not upshift it either"
  );
}

/// 400 kbps throughout: even the lowest rung does not fit, and the oracle
/// forwards the bottom rung rather than nothing.
#[test]
fn the_bottom_rung_is_the_floor() {
  let sets = [set(1, &[1000, 2000])];
  let mut oracle = tuned("400:60000");

  let selection = oracle.decide(&input(4, 2000, &sets));
  assert_eq!(selection[&1], Some(1000));
}

/// The rung is clamped to each set's own ladder: a short set serves its top
/// track, a tall one the rung.
#[test]
fn the_rung_is_clamped_to_each_sets_own_ladder() {
  let sets = [set(1, &[500]), set(2, &[500, 1000, 2000, 3000])];
  let mut oracle = tuned("4000:60000");

  let selection = oracle.decide(&input(1, 500, &sets));
  assert_eq!(
    selection[&1],
    Some(500),
    "the short set has no rung to climb to"
  );
  assert_eq!(selection[&2], Some(3000));
}

/// The schedule is predestined: the decision is a function of the schedule's
/// clock alone, so the same moment returns the same rung whatever the group
/// id, whatever the reason (a mid-group sample, a timeout) and whatever the
/// transport thought the path was worth.
#[test]
fn the_decision_is_a_function_of_the_clock_alone() {
  let sets = [set(1, &[1000, 2000, 3000])];
  for (group_id, estimate, reason) in [
    (0, 0, DecisionReason::GroupBoundary),
    (7, 999_999, DecisionReason::PeriodicSample),
    (42, 0, DecisionReason::StreamTimeout),
  ] {
    let mut oracle = tuned("1000:2000;2000:2000;3000:2000");
    let mut reask = input(group_id, 1000, &sets);
    reask.bandwidth_estimate_kbps = estimate;
    reask.reason = reason;
    assert_eq!(
      oracle.decide(&input(0, 0, &sets))[&1],
      Some(1000),
      "the anchor's own window"
    );
    assert_eq!(
      oracle.decide(&reask)[&1],
      Some(1000),
      "1.0 s is the 1 Gbps step, whatever the group id"
    );
  }
}

/// A set with no subscriber, or no members, forwards nothing: a paused set
/// frees the budget, and an empty one has nothing to forward.
#[test]
fn a_set_with_no_subscriber_or_no_members_forwards_nothing() {
  let mut paused = set(2, &[1000, 2000]);
  paused.active = false;
  let empty = set(3, &[]);
  let sets = [set(1, &[1000]), paused, empty];
  let mut oracle = tuned("2000:60000");

  let selection = oracle.decide(&input(1, 0, &sets));
  assert_eq!(selection[&1], Some(1000));
  assert_eq!(
    selection[&2], None,
    "a paused set frees the budget by forwarding nothing"
  );
  assert_eq!(
    selection[&3], None,
    "a set with no members has nothing to forward"
  );
}

/// 3 Gbps for the first quarter, 1 Gbps for the rest: the rung falls once the
/// window has walked past the rise, and the group it overtook — still in
/// flight at the old rung — keeps its delivery. The schedule is predestined,
/// so the selection carries no supersede request: reaching back would flip a
/// delivered rung away from it.
#[test]
fn a_downshift_supersedes_nothing() {
  let sets = [set(1, &[1000, 2000, 3000])];
  let mut oracle = tuned("3000:250;1000:4750");

  let first = oracle.decide(&input(1, 0, &sets));
  assert_eq!(
    first[&1],
    Some(2000),
    "the anchor's window straddles the drop: the mean holds 2000"
  );

  let second = oracle.decide(&input(2, 500, &sets));
  assert_eq!(second[&1], Some(1000), "the rung fell");
  assert!(
    second.supersede.is_empty(),
    "the overtaken group finishes on the member it was carrying"
  );
}

/// No `oracle.steps` tuning, no schedule: the bottom rung is pinned, and the
/// log says so rather than the oracle silently becoming a different algorithm.
#[test]
fn no_schedule_pins_the_bottom_rung() {
  let sets = [set(1, &[1000, 2000])];
  let mut oracle = OracleAlgorithm::from_tuning(&Tuning::empty());

  let selection = oracle.decide(&input(3, 1000, &sets));
  assert_eq!(
    selection[&1],
    Some(1000),
    "no steps, no price: the bottom rung holds"
  );
}

/// The trace's t0 is where the schedule begins, and a decision reads the
/// trace's own time from it: the run's bring-up before t0 is not a lead-in
/// the oracle can use, because the schedule has no step there.
#[test]
fn the_schedule_begins_at_the_traces_t0() {
  let sets = [set(1, &[1000, 2000, 3000])];
  // The trace opens with a 9 s warmup and begins 6 s into the host's clock:
  // a decision at 6.5 s is 0.5 s of the trace, inside the warmup.
  let mut oracle = tuned_at("9000:9000;1000:2000;2000:2000;3000:2000", 6000);
  assert_eq!(
    oracle.decide(&input(0, 6000, &sets))[&1],
    Some(3000),
    "the trace's first window is the warmup"
  );
  let selection = oracle.decide(&input(1, 6500, &sets));
  assert_eq!(
    selection[&1],
    Some(3000),
    "half a second into the trace is still the warmup"
  );
}

/// The last capacity holds past the schedule's end: a window beyond it is
/// priced at the last step's rate, which is the path the run leaves behind.
#[test]
fn the_last_capacity_holds_past_the_schedules_end() {
  // 3 Gbps for 1 s: 1.5 s is inside the schedule, 4.5 s is past its end.
  let sets = [set(1, &[1000, 2000, 3000])];
  let mut oracle = tuned("3000:1000");

  assert_eq!(oracle.decide(&input(3, 1500, &sets))[&1], Some(3000));
  assert_eq!(
    oracle.decide(&input(9, 4500, &sets))[&1],
    Some(3000),
    "past the end"
  );
}

/// The window is the one the group is delivered in, not one derived from its
/// id: group ids are the publisher's and start at the live edge (group ~5 on
/// this testbed), so an id-indexed window read the schedule 4-5 groups in the
/// past — the measured runs' decisions lagged the delivery by that much.
#[test]
fn the_window_is_the_delivery_window_not_the_group_id() {
  // 1 Gbps for 2 s, then 3 Gbps. The subscriber joins at group 5, 9 s in:
  // its decision reads the window at 9 s of the trace — the 3 Gbps step —
  // not the 1 Gbps the group id would have implied.
  let sets = [set(1, &[1000, 2000, 3000])];
  let mut oracle = tuned("1000:2000;3000:2000");
  assert_eq!(
    oracle.decide(&input(5, 9000, &sets))[&1],
    Some(3000),
    "the live edge's first group reads the capacity at its own delivery"
  );
  assert_eq!(
    oracle.decide(&input(6, 9500, &sets))[&1],
    Some(3000),
    "half a second later, still the same step"
  );
}

/// `oracle.goodput` is the application's share of the link, in percent: the
/// rung is priced at the window's mean times it. REAL01's case in miniature:
/// a 2456 kbps step against a 2441 kbps rung is a 15 kbps margin, which the
/// raw mean covers and the path does not.
#[test]
fn the_goodput_fraction_prices_the_rung_below_the_raw_mean() {
  let sets = [set(1, &[1000, 1672, 2441])];
  let mut raw = OracleAlgorithm::from_tuning(&Tuning::from_pairs([
    "oracle.steps=2456:60000",
    "oracle.group_ms=500",
  ]));
  assert_eq!(
    raw.decide(&input(0, 0, &sets))[&1],
    Some(2441),
    "2456 covers 2441 by 15 kbps"
  );

  let mut discounted = OracleAlgorithm::from_tuning(&Tuning::from_pairs([
    "oracle.steps=2456:60000",
    "oracle.group_ms=500",
    "oracle.goodput=90",
  ]));
  assert_eq!(
    discounted.decide(&input(0, 0, &sets))[&1],
    Some(1672),
    "2456 × 0.90 = 2210 does not"
  );
}

/// `oracle.group_ms` is the window the schedule is read in: at a 1 s cadence,
/// the window at 2.0 s is [2.0, 3.0) s of the schedule's time.
#[test]
fn the_group_window_is_a_knob() {
  let steps = format!("oracle.steps={}", "1000:2000;2000:2000;3000:2000");
  let sets = [set(1, &[1000, 2000, 3000])];
  let mut oracle = OracleAlgorithm::from_tuning(&Tuning::from_pairs([
    steps.as_str(),
    "oracle.group_ms=1000",
  ]));

  assert_eq!(
    oracle.decide(&input(0, 0, &sets))[&1],
    Some(1000),
    "the trace's first window"
  );
  let selection = oracle.decide(&input(2, 2000, &sets));
  assert_eq!(
    selection[&1],
    Some(2000),
    "2.0 s is [2.0, 3.0) s at a 1 s cadence"
  );
}
