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

//! The tier state machine behind the backpressure algorithm, with the
//! constants that define its response.
//!
//! It is deliberately a pure function of `(state, observation)`: the whole
//! point of the algorithm is that it needs no bandwidth estimate, so it has to
//! be exhaustively testable by feeding it a depth sequence. Nothing in here
//! touches the transport or the host.

/// Depth at or below which the connection is considered uncongested.
const DEPTH_TARGET: u64 = 1;

/// Depth at or above which the connection is congested and the tier drops
/// immediately, without waiting for a streak.
const DOWNSHIFT_DEPTH: u64 = 2;

/// Consecutive uncongested groups required before raising the tier. One group
/// of slack is a fluctuation; five is a trend.
const UPSHIFT_GOP_STREAK: u64 = 5;

/// Consecutive congested groups during cooldown, each deeper than the last,
/// before dropping the tier again.
const COOLDOWN_HIGH_STREAK: u64 = 2;

/// One group's worth of congestion evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Observation {
  /// Forwarding streams still open across the connection's active sets, plus
  /// the streams reset for delivery timeout since the previous observation.
  pub depth: u64,
  /// Whether at least one stream was reset for delivery timeout since the
  /// previous observation. Stronger than depth on its own: the transport
  /// dropped objects it had already committed to.
  pub timed_out: bool,
}

impl Observation {
  pub(crate) fn new(depth: u64, timeouts_since_last: u64) -> Self {
    Self {
      depth: depth + timeouts_since_last,
      timed_out: timeouts_since_last > 0,
    }
  }
}

/// Why the tier dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cause {
  /// Depth reached `DOWNSHIFT_DEPTH` outside cooldown.
  Depth,
  /// During cooldown, depth kept rising above the depth recorded at the last
  /// downshift.
  RisingDepth,
  /// During cooldown, a stream timed out on the tier that was just tried.
  Timeout,
}

/// What one observation did to the tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
  /// Nothing: the tier stayed where it was.
  Held,
  /// Depth returned to the target and the settle-down period ended.
  CooldownExit,
  /// The tier rose after `UPSHIFT_GOP_STREAK` uncongested groups.
  Upshift { from: usize, to: usize },
  /// The tier fell.
  Downshift {
    from: usize,
    to: usize,
    cause: Cause,
  },
}

/// The connection's position on the quality ladder.
#[derive(Debug, Default)]
pub(crate) struct TierState {
  /// Index into the ascending tier ladder; `0` is the lowest quality.
  current_index: usize,
  /// Consecutive groups seen at or below `DEPTH_TARGET`.
  clear_streak: u64,
  /// Depth recorded at the last downshift; `Some` means "in cooldown".
  post_downshift_depth: Option<u64>,
  /// Consecutive cooldown groups that were deeper than their predecessor.
  cooldown_high_streak: u64,
}

impl TierState {
  /// The tier to serve, as an index into a ladder sorted low to high.
  pub(crate) fn tier(&self) -> usize {
    self.current_index
  }

  /// Whether the machine is in the settle-down period after a downshift.
  #[cfg(test)]
  pub(crate) fn in_cooldown(&self) -> bool {
    self.post_downshift_depth.is_some()
  }

  /// Cap the tier at the longest ladder the connection has. Without this the
  /// index drifts above every set's ladder while all sets already serve their
  /// top track, and a downshift then changes nothing for several groups.
  pub(crate) fn clamp_to(&mut self, max_index: usize) {
    self.current_index = self.current_index.min(max_index);
  }

  /// Feed one group's observation, advancing the tier.
  pub(crate) fn observe(&mut self, observation: Observation) -> Event {
    if observation.depth <= DEPTH_TARGET {
      return self.observe_clear();
    }
    match self.post_downshift_depth {
      Some(reference) => self.observe_cooldown(observation, reference),
      None => self.observe_congested(observation.depth),
    }
  }

  /// An uncongested group: leave cooldown if we were in it, and count the
  /// streak towards an upshift.
  fn observe_clear(&mut self) -> Event {
    let exited = self.post_downshift_depth.take().is_some();
    if exited {
      self.cooldown_high_streak = 0;
    }
    self.clear_streak += 1;
    if self.clear_streak < UPSHIFT_GOP_STREAK {
      return if exited {
        Event::CooldownExit
      } else {
        Event::Held
      };
    }
    let from = self.current_index;
    self.current_index = from + 1;
    self.clear_streak = 0;
    Event::Upshift {
      from,
      to: self.current_index,
    }
  }

  /// A congested group outside cooldown: drop one tier and start settling down.
  /// The index never goes below zero, because the lowest tier is all this
  /// algorithm can offer.
  fn observe_congested(&mut self, depth: u64) -> Event {
    self.clear_streak = 0;
    if depth < DOWNSHIFT_DEPTH || self.current_index == 0 {
      return Event::Held;
    }
    let from = self.current_index;
    self.current_index = from - 1;
    self.post_downshift_depth = Some(depth);
    self.cooldown_high_streak = 0;
    Event::Downshift {
      from,
      to: self.current_index,
      cause: Cause::Depth,
    }
  }

  /// A congested group during cooldown: drop again if the tier just tried
  /// already lost a stream, or if the depth keeps climbing.
  fn observe_cooldown(&mut self, observation: Observation, reference: u64) -> Event {
    let depth = observation.depth;
    self.clear_streak = 0;
    self.post_downshift_depth = Some(depth);

    if observation.timed_out && self.current_index > 0 {
      self.current_index -= 1;
      self.cooldown_high_streak = 0;
      return Event::Downshift {
        from: self.current_index + 1,
        to: self.current_index,
        cause: Cause::Timeout,
      };
    }

    if depth <= reference {
      self.cooldown_high_streak = 0;
      return Event::Held;
    }

    self.cooldown_high_streak += 1;
    if self.cooldown_high_streak < COOLDOWN_HIGH_STREAK || self.current_index == 0 {
      return Event::Held;
    }
    self.current_index -= 1;
    self.cooldown_high_streak = 0;
    Event::Downshift {
      from: self.current_index + 1,
      to: self.current_index,
      cause: Cause::RisingDepth,
    }
  }
}
