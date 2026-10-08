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

//! The state machine behind the budget algorithm: one rung index per switching
//! set, moved by how much of that set's latency budget its own backlog has
//! used.
//!
//! Pure by construction — a function of `(state, signals, parameters, now)` —
//! so every threshold in here is reachable from a unit test without a
//! transport, a clock, or waiting for a 1.5 s window to pass.

use crate::Tuning;

/// Occupancy of the buffer below which the set is healthy.
const SAFE_FACTOR: f64 = 0.2;

/// Occupancy of the buffer above which a discard is imminent.
const DANGER_FACTOR: f64 = 0.7;

/// Consecutive healthy groups with a queue that is not growing before the set
/// climbs one rung.
const UPSHIFT_SAFE_STREAK: usize = 3;

/// Consecutive dangerous groups before a downshift; one makes it immediate.
const DOWNSHIFT_DANGER_STREAK: usize = 1;

/// Consecutive groups of timeouts that forces a downshift on its own.
const TIMEOUT_DOWNSHIFT_STREAK: usize = 3;

/// How long the tier holds after it last moved.
///
/// The old rung's streams keep draining for about one buffer's worth of time
/// after a change, so a sample taken inside that window measures the drain
/// rather than the new state — and holding every choice for at least the
/// subscriber's buffer guarantees the choice outlives the buffer it was made
/// for, which is what keeps the subscriber's own playout stable.
const DRAIN_WINDOW_MS: u64 = 1500;

/// The thresholds, and how long the tier holds after moving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BnbParams {
  /// Backlog / buffer below which the set is healthy.
  pub safe_factor: f64,
  /// Backlog / buffer above which a discard is imminent.
  pub danger_factor: f64,
  /// Consecutive healthy, non-growing groups before a climb.
  pub upshift_safe_streak: usize,
  /// Consecutive dangerous groups before a drop; 1 is immediate.
  pub downshift_danger_streak: usize,
  /// Consecutive groups of timeouts that forces a drop.
  pub timeout_downshift_streak: usize,
  /// How long the tier holds after a change, in ms.
  pub drain_window_ms: u64,
}

impl Default for BnbParams {
  fn default() -> Self {
    Self {
      safe_factor: SAFE_FACTOR,
      danger_factor: DANGER_FACTOR,
      upshift_safe_streak: UPSHIFT_SAFE_STREAK,
      downshift_danger_streak: DOWNSHIFT_DANGER_STREAK,
      timeout_downshift_streak: TIMEOUT_DOWNSHIFT_STREAK,
      drain_window_ms: DRAIN_WINDOW_MS,
    }
  }
}

impl BnbParams {
  /// The defaults, with whatever the host was given for the `bnb.` keys.
  pub fn from_tuning(tuning: &Tuning) -> Self {
    let defaults = Self::default();
    Self {
      safe_factor: tuning.f64("bnb.safe_factor", defaults.safe_factor),
      danger_factor: tuning.f64("bnb.danger_factor", defaults.danger_factor),
      upshift_safe_streak: tuning.usize("bnb.up_streak", defaults.upshift_safe_streak),
      downshift_danger_streak: tuning.usize("bnb.down_streak", defaults.downshift_danger_streak),
      timeout_downshift_streak: tuning
        .usize("bnb.timeout_streak", defaults.timeout_downshift_streak),
      drain_window_ms: tuning.f64("bnb.drain_window_ms", defaults.drain_window_ms as f64) as u64,
    }
  }
}

/// What one set looked like at this decision.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Signals {
  /// Streams of this set still open.
  pub depth: u64,
  /// How far the set's oldest open stream is behind one group period: its age
  /// minus the observed group span. A group whose stream lives exactly one span
  /// is a healthy pipeline, so this is the part that piled up.
  pub backlog_ms: u64,
  /// The subscriber's reorder buffer for this set, its latency budget; `0`
  /// means it negotiated nothing and the set has no deadline to miss.
  pub budget_ms: u64,
  /// Streams of this set reset for delivery timeout since the last decision.
  pub timeouts: u64,
}

/// What a step did, for the log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
  Hold,
  /// The queue is healthy and was not growing.
  Upshift,
  /// The budget is nearly spent; a discard is imminent.
  DownshiftDanger,
  /// The queue is in the middle band and growing.
  DownshiftAccumulation,
  /// Streams of this set are being given up on even though the queue drained.
  DownshiftTimeout,
  /// The tier was held because it moved less than a drain window ago.
  HoldInDrain,
}

/// One set's position, and the streaks that decide whether it moves.
#[derive(Debug, Default)]
pub(crate) struct SetState {
  /// Index into the set's ascending ladder; `0` is the lowest rung.
  tier: usize,
  /// Depth at the previous decision, for the "is the queue growing" test.
  previous_depth: u64,
  /// Consecutive healthy, non-growing groups.
  safe_streak: usize,
  /// Consecutive dangerous groups.
  down_streak: usize,
  /// The timeout veto: climbs are blocked while this is above zero, and a
  /// downshift fires at `timeout_downshift_streak`.
  timeout_streak: usize,
  /// When the tier last moved, on the host's clock.
  last_change_ms: Option<u64>,
}

impl SetState {
  /// The rung to serve, clamped to the set's ladder.
  pub(crate) fn tier(&self, max_index: usize) -> usize {
    self.tier.min(max_index)
  }

  /// The current timeout veto, for the log line.
  pub(crate) fn timeout_streak_for_log(&self) -> usize {
    self.timeout_streak
  }

  /// Feed one decision's signals and move the rung.
  pub(crate) fn step(
    &mut self,
    max_index: usize,
    signals: &Signals,
    params: &BnbParams,
    now_ms: u64,
  ) -> Action {
    let delta_depth = signals.depth.saturating_sub(self.previous_depth);
    self.previous_depth = signals.depth;

    // Timeouts are their own channel: they extend the veto when they arrive and
    // decay by one per clean group, so a burst keeps blocking climbs for a
    // while after the queue it caused has drained.
    let timeout_streak = if signals.timeouts > 0 {
      self.timeout_streak + signals.timeouts as usize
    } else {
      self.timeout_streak.saturating_sub(1)
    };
    self.timeout_streak = timeout_streak;

    if self
      .last_change_ms
      .is_some_and(|at| now_ms.saturating_sub(at) < params.drain_window_ms)
    {
      self.safe_streak = 0;
      self.down_streak = 0;
      // A sustained timeout is still the emergency: it means the rung is
      // unsustainable even while the queue looks like it is draining.
      if timeout_streak >= params.timeout_downshift_streak && self.tier > 0 {
        self.tier -= 1;
        self.timeout_streak = 0;
        self.last_change_ms = Some(now_ms);
        return Action::DownshiftTimeout;
      }
      return Action::HoldInDrain;
    }

    let action = self.ladder_step(max_index, signals, params, delta_depth);
    if action != Action::Hold {
      self.last_change_ms = Some(now_ms);
    }
    action
  }

  /// The ladder itself, outside the drain window.
  fn ladder_step(
    &mut self,
    max_index: usize,
    signals: &Signals,
    params: &BnbParams,
    delta_depth: u64,
  ) -> Action {
    // A subscriber that negotiated no buffer has no latency budget, so there is
    // no occupancy to be dangerous at: only the climb can move the rung.
    let (safe_ms, danger_ms) = if signals.budget_ms == 0 {
      (u64::MAX, u64::MAX)
    } else {
      (
        (signals.budget_ms as f64 * params.safe_factor) as u64,
        (signals.budget_ms as f64 * params.danger_factor) as u64,
      )
    };

    if signals.backlog_ms > danger_ms {
      self.down_streak += 1;
      if self.down_streak >= params.downshift_danger_streak && self.tier > 0 {
        self.tier -= 1;
        self.safe_streak = 0;
        self.timeout_streak = 0;
        self.down_streak = 0;
        return Action::DownshiftDanger;
      }
      // At the bottom rung the streak is meaningless; keep it drained rather
      // than armed, so the first group that can act, acts.
      if self.tier == 0 {
        self.down_streak = 0;
      }
      Action::Hold
    } else if self.timeout_streak >= params.timeout_downshift_streak {
      // Sustained timeouts: the rung is unsustainable even though the queue has
      // drained, and a drain is exactly what hides that from the other signals.
      if self.tier > 0 {
        self.tier -= 1;
        self.safe_streak = 0;
        self.timeout_streak = 0;
        self.down_streak = 0;
        return Action::DownshiftTimeout;
      }
      Action::Hold
    } else if signals.backlog_ms >= safe_ms {
      self.safe_streak = 0;
      self.down_streak = 0;
      if delta_depth > 0 && self.tier > 0 {
        self.tier -= 1;
        return Action::DownshiftAccumulation;
      }
      Action::Hold
    } else if self.timeout_streak > 0 {
      // Healthy, but a recent timeout vetoes the probe.
      self.safe_streak = 0;
      self.down_streak = 0;
      Action::Hold
    } else {
      self.down_streak = 0;
      if delta_depth > 0 || self.tier >= max_index {
        self.safe_streak = 0;
        Action::Hold
      } else if self.safe_streak + 1 >= params.upshift_safe_streak {
        self.tier += 1;
        self.safe_streak = 0;
        Action::Upshift
      } else {
        self.safe_streak += 1;
        Action::Hold
      }
    }
  }
}
