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

//! The perfect-knowledge oracle, run on the testbed.
//!
//! The ceiling the adaptive algorithms are measured against is a sender that
//! knows the trace: the oracle is granted the whole capacity schedule in
//! advance — the applied `tc` steps of the bandwidth the measured runs saw,
//! handed over as a tuning value so the crate never touches the filesystem —
//! and it prices every group from the schedule alone. Window g is steps
//! [g·G, (g+1)·G) from the schedule's origin, and the rung is the highest one
//! whose nominal rate the window's mean capacity covers, floored at the bottom
//! rung (nothing cheaper exists). No estimate is read, no depth is counted, no
//! evidence is taken: the decision for a group is a function of its index
//! alone, which is what makes the schedule predestined — a mid-group re-ask
//! returns the answer the boundary already recorded, and a downshift between
//! consecutive windows leaves the overtaken groups in flight, finishing on the
//! member they were carrying, which is their own predestined rung. Reaching
//! back would flip a delivered rung away from the schedule.
//!
//! The windows are the trace's own: the host hands over `oracle.t0_ms`, the
//! trace's t0 on the Unix epoch, and passes the same instant as
//! `now_unix_ms`, so a decision reads `[now_unix_ms - t0_ms, ... + G)` — the
//! 500 ms of the trace that starts now, which is the bandwidth the group's
//! bytes will cross the link in. No group id enters it, and no anchor is
//! needed: the oracle knows what time it is, and that time is the trace's.
//!
//! It exists to be measured, not to be good: the schedule is fixed by the
//! trace, the run is deterministic, and one run per trace reports the exact
//! outcome — the latency, freezes and QoE of delivering the predestined
//! schedule through the real path, scored with the same metric code as the
//! measured runs.

use std::sync::OnceLock;

use crate::{AbrAlgorithm, AbrAlgorithmFactory, AbrInput, Selection, Tuning};

#[cfg(test)]
mod tests;

mod schedule;

use schedule::Schedule;

/// MOQtail's oracle: the rung each group window's mean capacity covers,
/// predestined from the trace. The next free id in the private range — 0xff03
/// stays parked for BNB, 0xff04 and above are the wasd versions.
pub const ORACLE_ALGORITHM_ID: u64 = crate::registry::PRIVATE_ALGORITHM_ID_BASE + 6;

/// The group window the schedule is read in, in ms: the ladder's GOP over its
/// frame rate (15 frames at 30 fps). The host passes it so a changed cadence
/// is visible in the run's record; the default is the ladder's own.
const DEFAULT_GROUP_MS: u64 = 500;

/// Factory for [`OracleAlgorithm`]: one instance per connection, named
/// `oracle` on the wire.
pub struct OracleAlgorithmFactory;

impl AbrAlgorithmFactory for OracleAlgorithmFactory {
  fn id(&self) -> u64 {
    ORACLE_ALGORITHM_ID
  }

  fn name(&self) -> &'static str {
    "oracle"
  }

  fn create(&self, tuning: &Tuning) -> Box<dyn AbrAlgorithm> {
    Box::new(OracleAlgorithm::from_tuning(tuning))
  }
}

/// The predestined walk: one rung per group, priced from the schedule alone.
pub struct OracleAlgorithm {
  /// The capacity schedule, parsed from the tuning when the instance is
  /// created. `None` only when the host named the oracle without handing it
  /// `oracle.steps`, and pins the bottom rung.
  schedule: OnceLock<Schedule>,
  /// The trace's t0 on the Unix epoch, in ms: where the schedule's clock
  /// begins, handed over as `oracle.t0_ms`. The host passes the same instant
  /// as `now_unix_ms`, so the difference is the trace's own time.
  t0_ms: u64,
  /// The steps as the host handed them over.
  steps_value: Option<String>,
  /// The group window the schedule is read in, in ms.
  group_ms: u64,
  /// The application's share of the link, in percent: the rung is priced at
  /// the window's mean times it. The tc steps name the link; the QUIC/UDP/IP
  /// framing and CUBIC's dynamics mean the application delivers less, so a
  /// rung priced at the raw mean can fit by arithmetic and overrun in
  /// reality. Handed over as `oracle.goodput`.
  goodput_pct: u64,
}

impl OracleAlgorithm {
  /// From the host's tuning: `oracle.steps` carries the applied capacity
  /// steps as `cap_kbps:duration_ms` pairs joined by `;`, cumulative from the
  /// trace's start, and `oracle.group_ms` the window the schedule is read in.
  pub fn from_tuning(tuning: &Tuning) -> Self {
    Self {
      schedule: OnceLock::new(),
      t0_ms: tuning.usize("oracle.t0_ms", 0) as u64,
      steps_value: tuning.str("oracle.steps").map(str::to_string),
      group_ms: tuning.usize("oracle.group_ms", DEFAULT_GROUP_MS as usize) as u64,
      goodput_pct: tuning.usize("oracle.goodput", 100) as u64,
    }
  }

  /// The rung the group being decided is priced at: the highest one whose
  /// nominal rate the mean capacity over the NEXT window covers, floored at
  /// the bottom rung.
  ///
  /// The oracle knows what time it is and what the bandwidth will be: the
  /// group is decided when its first Object reaches the relay, and its bytes
  /// cross the link over the following `G` (500 ms). So the window is
  /// `[now, now + G)` on the schedule's clock, and the decision is the mean
  /// of that window — the bandwidth the group will actually be delivered
  /// into. No group id enters it (the publisher numbers groups from the live
  /// edge, and an id-indexed window read the schedule 4-5 groups in the
  /// past).
  ///
  /// The mean is discounted by the goodput fraction: the tc steps name the
  /// link, and the application delivers less of it (QUIC/UDP/IP framing,
  /// CUBIC's queue drops and backoff). On REAL01 the ladder's 2441 kbps rung
  /// against a 2456 kbps step has 15 kbps of margin — under 1 % — so a rung
  /// priced at the raw mean fit by arithmetic and overran in reality, and the
  /// group froze. At 0.90 the 2441 rung needs 2712 kbps of link and the
  /// oracle takes 1672 there instead, cleanly.
  fn rung(&self, schedule: &Schedule, from_ms: u64, group_ms: u64) -> f64 {
    let mean = schedule.mean_kbps(from_ms, from_ms.saturating_add(group_ms));
    mean * (self.goodput_pct as f64 / 100.0)
  }

  /// Serve every set the schedule prices for: the member at the rung, clamped
  /// to each set's own ladder; a set with no subscriber or no members forwards
  /// nothing.
  fn serve(&self, input: &AbrInput, cap_kbps: f64) -> Selection {
    let mut selection = Selection::with_capacity(input.sets.len());
    for set in input.sets {
      let track = if set.active && !set.members.is_empty() {
        set
          .member_up_to(cap_kbps.floor().max(0.0) as u64)
          .or_else(|| set.lowest_member())
      } else {
        None
      };
      selection.insert(set.id, track);
    }
    selection
  }
}

impl AbrAlgorithm for OracleAlgorithm {
  fn decide(&mut self, input: &AbrInput) -> Selection {
    // The schedule's clock is the trace's clock: `oracle.t0_ms` is the
    // trace's t0 (the first applied tc step) on the Unix epoch, and the host
    // passes the same instant as `now_unix_ms`. The schedule is therefore read
    // at `now_unix_ms - t0_ms` — the time on the trace's own timeline — and no
    // anchor is needed: the oracle knows what time it is, and that time is the
    // trace's.
    //
    // The join is on the Unix epoch, not on `now_ms`: `now_ms` is monotonic
    // and means nothing outside the host, so `now_ms - t0_ms` mixed two clock
    // domains (the measured runs did exactly that and read a window 26 s into
    // a 60 s trace at the 24 s mark). Re-basing at the first decision instead
    // put the schedule's origin at the first group rather than at the trace's
    // t0, which was the earlier version's 3.5 s lag.
    let t0_ms = self.t0_ms;
    let schedule = self.schedule.get_or_init(|| match self.steps_value.take() {
      Some(value) => Schedule::from_steps_value(&value),
      None => {
        tracing::warn!("SSTS oracle: no oracle.steps tuning; the bottom rung is pinned");
        Schedule::default()
      }
    });
    // The window this group's delivery starts in: the group is decided when
    // its first Object reaches the relay, so the window is [now, now + G) on
    // the trace's clock — the bandwidth its bytes will actually cross the
    // link in. No group id enters it (the publisher numbers groups from the
    // live edge, and an id-indexed window read the schedule groups in the
    // past).
    let from_ms = input.now_unix_ms.saturating_sub(t0_ms);
    let cap_kbps = self.rung(schedule, from_ms, self.group_ms);
    tracing::debug!(
      client_id = input.client_id,
      group_id = input.group_id,
      trace_ms = from_ms,
      cap_kbps,
      "SSTS oracle: window capacity for this group"
    );
    self.serve(input, cap_kbps)
  }
}
