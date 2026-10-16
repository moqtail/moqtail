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

//! The tier state machine behind wasd, and the reading that moves it.
//!
//! Pure again: `(state, sample) -> event`, no clock, no transport. There is
//! one reading now — the depth at the tier being delivered — fed by every
//! sample the host takes, at a group boundary or mid-group alike. What a
//! sample is allowed to do depends on one fact the state itself tracks:
//! whether it is the first reading of a new group, which is the only moment
//! an upshift can land before the relay's forward gate records that group's
//! track decision.

use crate::algorithms::asd::state::DEPTH_TARGET;
use crate::algorithms::asd::state::DOWNSHIFT_DEPTH;
use crate::algorithms::asd::state::UPSHIFT_GOP_STREAK;

/// Slow start: one uncongested group is enough to try the next tier.
pub(crate) const SLOW_START_UPSHIFT_STREAK: u64 = 1;

/// Consecutive mid-group depth samples, at the downshift depth or above,
/// before the monitor acts. A single such reading is normal right after a new
/// group's stream opens, while the previous group's FIN-ACK is still in
/// flight; congestion shows it on every reading until it is dealt with. The
/// streak re-arms when it fires, so one burst cannot chain.
pub(crate) const DOWNSHIFT_SUBGROUP_READS: u32 = 2;

/// What one depth sample did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
  /// Nothing.
  Held,
  Upshift {
    from: usize,
    to: usize,
  },
  Downshift {
    from: usize,
    to: usize,
  },
}

/// The connection's position on the quality ladder.
#[derive(Debug, Default)]
pub(crate) struct LadderState {
  /// Index into the ascending tier ladder; `0` is the lowest quality.
  current_index: usize,
  /// Consecutive uncongested group boundaries at or below `DEPTH_TARGET`.
  clear_streak: u64,
  /// Consecutive mid-group samples at or above `DOWNSHIFT_DEPTH`.
  monitor_streak: u32,
  /// How many consecutive congested mid-group samples the monitor waits for
  /// before acting.
  monitor_reads: u32,
  /// Until the ladder's top or the first sustained congestion, whichever comes first.
  exploring: bool,
  /// The group_id of the previous sample, ensuring upshifts are only evaluated
  /// when a new group is being decided before `record_group_decision` locks it.
  last_group_id: Option<u64>,
}

impl LadderState {
  /// A connection at the bottom tier, exploring.
  pub(crate) fn new() -> Self {
    Self {
      current_index: 0,
      clear_streak: 0,
      monitor_streak: 0,
      monitor_reads: DOWNSHIFT_SUBGROUP_READS,
      exploring: true,
      last_group_id: None,
    }
  }

  /// The same machine with a different patience in the monitor. Zero is clamped
  /// to one: a monitor that fires on zero readings fires on every sample, which
  /// is not a monitor but a reaction.
  pub(crate) fn new_with_reads(reads: u32) -> Self {
    let mut state = Self::new();
    state.monitor_reads = reads.max(1);
    state
  }

  /// The tier to serve.
  pub(crate) fn tier(&self) -> usize {
    self.current_index
  }

  /// Whether the connection is still exploring the ladder.
  pub(crate) fn exploring(&self) -> bool {
    self.exploring
  }

  /// Cap the tier at the longest ladder the connection has, so a downshift
  /// lands on a track that exists the group after it was decided.
  pub(crate) fn clamp_to(&mut self, max_index: usize) {
    self.current_index = self.current_index.min(max_index);
  }

  /// Evaluate one depth sample for `group_id`.
  ///
  /// The first sample for a new `group_id` (`is_new_group`) arrives when the
  /// forward gate requests a decision before opening the new group's stream.
  /// Subsequent samples for the same `group_id` arrive while the group is in
  /// flight.
  pub(crate) fn on_sample(
    &mut self,
    depth: u64,
    group_id: u64,
    may_act: bool,
    max_index: usize,
  ) -> Step {
    let is_new_group = self.last_group_id != Some(group_id);
    self.last_group_id = Some(group_id);

    if is_new_group {
      // At the group boundary (before the new group's stream opens), a depth
      // <= DEPTH_TARGET means the previous group drained on time.
      if depth <= DEPTH_TARGET {
        self.clear_streak += 1;
        let needed = if self.exploring {
          SLOW_START_UPSHIFT_STREAK
        } else {
          UPSHIFT_GOP_STREAK
        };
        if self.clear_streak >= needed {
          let from = self.current_index;
          let to = (from + 1).min(max_index);
          self.current_index = to;
          self.clear_streak = 0;
          if self.exploring && self.current_index >= max_index && may_act {
            self.exploring = false;
          }
          if to > from {
            return Step::Upshift { from, to };
          }
        }
        return Step::Held;
      }

      // Congested at the group boundary: reset streaks and end exploration.
      // Two streams of the delivered tier still open when a group's first
      // Object arrives means two groups behind — the handoff overlap cannot
      // produce that, because the new group's stream has not opened yet — so
      // the boundary needs no streak to be certain.
      self.clear_streak = 0;
      self.monitor_streak = 0;
      if self.exploring && depth >= DOWNSHIFT_DEPTH {
        self.exploring = false;
      }
      if depth >= DOWNSHIFT_DEPTH && self.current_index > 0 {
        let from = self.current_index;
        self.current_index -= 1;
        return Step::Downshift {
          from,
          to: self.current_index,
        };
      }
      return Step::Held;
    }

    // Intra-group sample (`!is_new_group`):
    // A single reading of `depth >= DOWNSHIFT_DEPTH` is normal right after the
    // new group's stream opens while the previous group's FIN-ACK is still in
    // flight. Only sustained congestion across `monitor_reads` consecutive
    // samples resets `clear_streak` and triggers a mid-group downshift.
    self.monitor_streak = if depth >= DOWNSHIFT_DEPTH {
      self.monitor_streak.saturating_add(1)
    } else {
      0
    };

    if !self.exploring
      && self.current_index > 0
      && self.monitor_streak >= self.monitor_reads
      && may_act
    {
      let from = self.current_index;
      self.current_index -= 1;
      self.monitor_streak = 0;
      self.clear_streak = 0;
      return Step::Downshift {
        from,
        to: self.current_index,
      };
    }

    Step::Held
  }
}
