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

//! The tier state machine behind wasd, and the readings that move it.
//!
//! Pure again: `(state, reading) -> event`, no clock, no transport. The two
//! readings are the two questions the algorithm asks — what the depth was at
//! the last group boundary, and what it is right now in the middle of one — and
//! they are separate types because they are allowed to do different things.

use crate::algorithms::asd::state::DEPTH_TARGET;
use crate::algorithms::asd::state::DOWNSHIFT_DEPTH;
use crate::algorithms::asd::state::UPSHIFT_GOP_STREAK;

/// Slow start: one uncongested group is enough to try the next tier.
pub(crate) const SLOW_START_UPSHIFT_STREAK: u64 = 1;

/// Consecutive depth readings inside one group, at the downshift depth or
/// above, before the monitor acts. A healthy group's tail shows one such
/// reading, because the next group's stream opens before the previous one has
/// finished draining; congestion shows it on every reading until it is dealt
/// with. The streak re-arms when it fires, so one burst cannot chain.
pub(crate) const DOWNSHIFT_SUBGROUP_READS: u32 = 2;

/// What a mid-transmission reading did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Monitor {
  /// Nothing: the readings have not been bad long enough, or the machine is not
  /// in a state where acting on them is allowed.
  Held,
  /// The depth inside the group stayed high long enough to stop waiting for the
  /// next group boundary.
  Downshift { from: usize, to: usize },
}

/// What a boundary reading did.
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
  /// Consecutive boundary readings at or below `DEPTH_TARGET`.
  clear_streak: u64,
  /// Consecutive mid-transmission readings at or above `DOWNSHIFT_DEPTH`.
  monitor_streak: u32,
  /// How many of them the monitor waits for. `DOWNSHIFT_SUBGROUP_READS` is the
  /// default; an operator whose media paces differently than the sample the
  /// default was tuned on can move it without moving the algorithm.
  monitor_reads: u32,
  /// Until the ladder's top or the first congestion, whichever comes first.
  exploring: bool,
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

  /// Congestion ends the exploration, whether or not a downshift was possible:
  /// a connection already at the bottom tier has still learned that the path is
  /// narrower than the ladder.
  pub(crate) fn note_congestion(&mut self, depth: u64) -> bool {
    if self.exploring && depth >= DOWNSHIFT_DEPTH {
      self.exploring = false;
      // Whatever the monitor had been counting belongs to the exploration. The
      // arm it just unlocked counts from here.
      self.monitor_streak = 0;
      return true;
    }
    false
  }

  /// The exploration ends at the top of the ladder too, and only once a set has
  /// a subscriber: while tracks are still being assigned the ladder itself is
  /// still growing, and calling that the top would end the exploration on a
  /// connection that has barely started.
  pub(crate) fn note_top_of_ladder(&mut self, max_index: usize, any_active: bool) -> bool {
    if self.exploring && self.current_index >= max_index && any_active {
      self.exploring = false;
      return true;
    }
    false
  }

  /// One group boundary: the walk the other tier algorithms know, with the
  /// exploration streak swapped in while the connection is exploring.
  pub(crate) fn on_group(&mut self, depth: u64) -> Step {
    if depth <= DEPTH_TARGET {
      self.clear_streak += 1;
      let needed = if self.exploring {
        SLOW_START_UPSHIFT_STREAK
      } else {
        UPSHIFT_GOP_STREAK
      };
      if self.clear_streak < needed {
        return Step::Held;
      }
      let from = self.current_index;
      self.current_index = from + 1;
      self.clear_streak = 0;
      return Step::Upshift {
        from,
        to: self.current_index,
      };
    }

    // Congested: the climb starts over, and a dip is answered by falling one
    // rung only when there is a rung to fall to.
    self.clear_streak = 0;
    self.monitor_streak = 0;
    if depth < DOWNSHIFT_DEPTH || self.current_index == 0 {
      return Step::Held;
    }
    let from = self.current_index;
    self.current_index = from - 1;
    Step::Downshift {
      from,
      to: self.current_index,
    }
  }

  /// One reading taken while a group is still being served.
  ///
  /// `may_act` is the host's answer to "is there anything to act on": the
  /// monitor does not fire on a connection whose sets are idle, and never
  /// interrupts the exploration, which needs whole groups to walk the ladder.
  pub(crate) fn on_tick(&mut self, depth: u64, may_act: bool) -> Monitor {
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
      // The same hygiene the boundary path applies: a dip has to earn a full
      // fresh clean streak before the ladder can be climbed back, instead of
      // bouncing on a streak carried over from before the dip.
      self.clear_streak = 0;
      return Monitor::Downshift {
        from,
        to: self.current_index,
      };
    }
    Monitor::Held
  }
}
