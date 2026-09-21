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

//! Backpressure with slow start: the same depth signal, reached faster.
//!
//! [`crate::algorithms::backpressure`] climbs one tier per five uncongested
//! groups from the moment the connection starts, which is the right caution
//! once the connection knows something about the path and the wrong caution
//! when it knows nothing: a connection that begins at the bottom tier and has
//! three times the bandwidth it needs still spends fifteen groups climbing.
//! This algorithm explores like TCP does — one tier per uncongested group
//! until the ladder's top or until the first sign of congestion — and then
//! behaves exactly like backpressure for the rest of the connection.
//!
//! The exploration is over, permanently, at whichever of these comes first:
//!
//! * depth reaches [`state::DOWNSHIFT_DEPTH`], which is the first evidence that
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
use crate::Selection;
use crate::algorithms::backpressure::state::DOWNSHIFT_DEPTH;
use crate::algorithms::backpressure::state::Event;
use crate::algorithms::backpressure::state::Observation;
use crate::algorithms::backpressure::state::TierState;
use crate::algorithms::backpressure::state::UPSHIFT_GOP_STREAK;

#[cfg(test)]
mod tests;

/// Slow start: one uncongested group is enough to try the next tier.
const SLOW_START_UPSHIFT_STREAK: u64 = 1;

/// MOQtail's backpressure with slow start: the second id in the range the
/// specification leaves to implementations.
pub const ENHANCED_BACKPRESSURE_ALGORITHM_ID: u64 = crate::registry::PRIVATE_ALGORITHM_ID_BASE + 1;

/// Factory for [`EnhancedBackpressureAlgorithm`]: one instance per connection.
pub struct EnhancedBackpressureAlgorithmFactory;

impl AbrAlgorithmFactory for EnhancedBackpressureAlgorithmFactory {
  fn id(&self) -> u64 {
    ENHANCED_BACKPRESSURE_ALGORITHM_ID
  }

  fn name(&self) -> &'static str {
    "enhanced_bp"
  }

  fn create(&self) -> Box<dyn AbrAlgorithm> {
    Box::new(EnhancedBackpressureAlgorithm::new())
  }
}

/// Backpressure's tier walk, with an exploration phase at the start.
pub struct EnhancedBackpressureAlgorithm {
  tier: TierState,
  /// Until the ladder's top or the first congestion, whichever comes first.
  exploring: bool,
}

impl EnhancedBackpressureAlgorithm {
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

impl Default for EnhancedBackpressureAlgorithm {
  fn default() -> Self {
    Self::new()
  }
}

impl AbrAlgorithm for EnhancedBackpressureAlgorithm {
  fn decide(&mut self, input: &AbrInput) -> Selection {
    let observation = Observation::new(
      input.active_stream_depth(),
      input.stream_timeouts_since_last_decision,
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
        "SSTS enhanced_bp: exploration over (congestion)"
      );
    }

    self.tier.set_upshift_streak(if self.exploring {
      SLOW_START_UPSHIFT_STREAK
    } else {
      UPSHIFT_GOP_STREAK
    });

    // The index is shared by the connection's sets, so it cannot be higher than
    // the longest ladder among them.
    let max_index = input
      .sets
      .iter()
      .map(|set| set.members.len())
      .max()
      .unwrap_or(1)
      .saturating_sub(1);

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
        "SSTS enhanced_bp: exploration over (top of the ladder)"
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
      "SSTS enhanced_bp: depth back at target, leaving cooldown"
    ),
    Event::Upshift { from, to } => tracing::debug!(
      client_id = input.client_id,
      group_id = input.group_id,
      from,
      to,
      depth,
      "SSTS enhanced_bp: UPSHIFT"
    ),
    Event::Downshift { from, to, cause } => tracing::debug!(
      client_id = input.client_id,
      group_id = input.group_id,
      from,
      to,
      depth,
      ?cause,
      "SSTS enhanced_bp: DOWNSHIFT"
    ),
  }
}
