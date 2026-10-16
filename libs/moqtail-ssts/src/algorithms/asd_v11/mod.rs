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

//! ASD v1.1 — backpressure with slow start: the same depth signal, reached
//! faster.
//!
//! [`crate::algorithms::asd`] climbs one tier per five uncongested
//! groups from the moment the connection starts, which is the right caution
//! once the connection knows something about the path and the wrong caution
//! when it knows nothing: a connection that begins at the bottom tier and has
//! three times the bandwidth it needs still spends fifteen groups climbing.
//! This algorithm explores like TCP does — one tier per uncongested group
//! until the ladder's top or until the first sign of congestion — and then
//! behaves exactly like ASD for the rest of the connection.
//!
//! The exploration is over, permanently, at whichever of these comes first:
//!
//! * depth reaches `DOWNSHIFT_DEPTH` (see [`crate::algorithms::asd`]), which
//!   is the first evidence that
//!   the ladder is taller than the path;
//! * the tier reaches the top of the longest ladder among the active sets,
//!   which is the evidence that there is nothing left to explore.
//!
//! It does not restart. A connection that drops to the bottom tier because of
//! a transient does not get to probe its way up one tier per group again: the
//! second climb is the cautious one, because by then the connection has seen
//! the path fail and the point of the streak is to not do that again.

use crate::AbrAlgorithm;
use crate::AbrAlgorithmFactory;
use crate::AbrInput;
use crate::DecisionReason;
use crate::Selection;
use crate::Tuning;
use crate::algorithms::asd::state::DOWNSHIFT_DEPTH;
use crate::algorithms::asd::state::Event;
use crate::algorithms::asd::state::Observation;
use crate::algorithms::asd::state::TierState;
use crate::algorithms::asd::state::UPSHIFT_GOP_STREAK;

#[cfg(test)]
mod tests;

/// Slow start: one uncongested group is enough to try the next tier.
const SLOW_START_UPSHIFT_STREAK: u64 = 1;

/// ASD v1.1, slow start on ASD's depth signal: the second id in the range the
/// specification leaves to implementations.
pub const ASD_V11_ALGORITHM_ID: u64 = crate::registry::PRIVATE_ALGORITHM_ID_BASE + 1;

/// Factory for [`AsdV11Algorithm`]: one instance per connection.
pub struct AsdV11AlgorithmFactory;

impl AbrAlgorithmFactory for AsdV11AlgorithmFactory {
  fn id(&self) -> u64 {
    ASD_V11_ALGORITHM_ID
  }

  fn name(&self) -> &'static str {
    "asd-v1.1"
  }

  fn create(&self, _tuning: &Tuning) -> Box<dyn AbrAlgorithm> {
    Box::new(AsdV11Algorithm::new())
  }
}

/// ASD's tier walk, with an exploration phase at the start.
pub struct AsdV11Algorithm {
  tier: TierState,
  /// Until the ladder's top or the first congestion, whichever comes first.
  exploring: bool,
}

impl AsdV11Algorithm {
  /// A connection that starts at the bottom tier and explores upward one tier
  /// per uncongested group.
  pub fn new() -> Self {
    Self {
      tier: TierState::new(),
      exploring: true,
    }
  }

  /// Whether the connection is still in its exploration phase.
  pub fn exploring(&self) -> bool {
    self.exploring
  }
}

impl Default for AsdV11Algorithm {
  fn default() -> Self {
    Self::new()
  }
}

impl AbrAlgorithm for AsdV11Algorithm {
  fn decide(&mut self, input: &AbrInput) -> Selection {
    // The index is shared by the connection's sets, so it cannot be higher than
    // the longest ladder among them.
    let max_index = input
      .sets
      .iter()
      .map(|set| set.members.len())
      .max()
      .unwrap_or(1)
      .saturating_sub(1);

    // A per-object sample re-evaluates the group in flight but is not a new
    // group: the slow-start and cautious streaks count uncongested groups, so
    // the state machine is walked only at a group boundary or on a stream
    // timeout (immediate congestion evidence). The tier in hand still serves
    // the group in flight.
    if input.reason == DecisionReason::PeriodicSample {
      self.tier.clamp_to(max_index);
      return crate::algorithms::serve_tier(input, self.tier.tier());
    }

    let observation = Observation::new(
      input.active_stream_depth(),
      input.total_stream_timeouts_since_last_decision(),
    );

    // Congestion ends the exploration whether or not the tier had anywhere to
    // fall: a connection already at the bottom tier has still learned that the
    // path is narrower than the ladder.
    if self.exploring && observation.depth >= DOWNSHIFT_DEPTH {
      self.exploring = false;
      tracing::debug!(
        client_id = input.client_id,
        group_id = input.group_id,
        depth = observation.depth,
        "SSTS asd-v1.1: exploration over (congestion)"
      );
    }

    self.tier.set_upshift_streak(if self.exploring {
      SLOW_START_UPSHIFT_STREAK
    } else {
      UPSHIFT_GOP_STREAK
    });

    let event = self.tier.observe(observation);
    log_event(input, observation.depth, self.exploring, &event);

    if let Event::Upshift { to, .. } = event
      && self.exploring
      && to >= max_index
      // Only count the top while a set has a subscriber: while tracks are still
      // being assigned the ladder itself is still growing, and calling that the
      // top would end the exploration on a connection that has barely started.
      && input.sets.iter().any(|set| set.active)
    {
      self.exploring = false;
      tracing::debug!(
        client_id = input.client_id,
        group_id = input.group_id,
        tier = to,
        "SSTS asd-v1.1: exploration over (top of the ladder)"
      );
    }

    self.tier.clamp_to(max_index);
    crate::algorithms::serve_tier(input, self.tier.tier())
  }
}

/// One line per tier change and per end of cooldown, naming the depth that
/// caused it. Held groups are silent: they are the common case and say nothing.
fn log_event(input: &AbrInput, depth: u64, exploring: bool, event: &Event) {
  match event {
    Event::Held => {}
    Event::CooldownExit => tracing::debug!(
      client_id = input.client_id,
      group_id = input.group_id,
      depth,
      exploring,
      "SSTS asd-v1.1: depth back at target, leaving cooldown"
    ),
    Event::Upshift { from, to } => tracing::debug!(
      client_id = input.client_id,
      group_id = input.group_id,
      from,
      to,
      depth,
      "SSTS asd-v1.1: UPSHIFT"
    ),
    Event::Downshift { from, to, cause } => tracing::debug!(
      client_id = input.client_id,
      group_id = input.group_id,
      from,
      to,
      depth,
      ?cause,
      "SSTS asd-v1.1: DOWNSHIFT"
    ),
  }
}
