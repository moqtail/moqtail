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

//! Backpressure-driven tier selection: how many streams are still in flight,
//! rather than how much bandwidth is estimated to be available.
//!
//! The connection holds one tier index, shared by all its sets. Every active
//! set forwards the member at that index, clamped to its own ladder, so the
//! algorithm always forwards something and all of a subscriber's tracks move
//! quality together. The index moves on stream accounting alone:
//!
//! * while the depth stays at `DEPTH_TARGET` or below for `UPSHIFT_GOP_STREAK`
//!   consecutive groups, the tier rises by one;
//! * when the depth reaches `DOWNSHIFT_DEPTH`, the tier falls by one and a
//!   settle-down period starts; during it, a stream that times out on the tier
//!   just tried, or depth that keeps rising for `COOLDOWN_HIGH_STREAK`
//!   consecutive groups, drops the tier again, and the period ends once depth
//!   is back at the target.
//!
//! See the README in this directory for the two things this gets wrong.

mod state;

use crate::{AbrAlgorithm, AbrAlgorithmFactory, AbrInput, Selection};

#[cfg(test)]
mod tests;

use state::{Event, Observation, TierState};

/// MOQtail's own tier selector. This is not a registered id: it is the first
/// value in the range the specification leaves to implementations, so
/// experimenting with it cannot collide with a future allocation. See
/// [`crate::registry::PRIVATE_ALGORITHM_ID_BASE`].
pub const BACKPRESSURE_ALGORITHM_ID: u64 = crate::registry::PRIVATE_ALGORITHM_ID_BASE;

/// Factory for [`BackpressureAlgorithm`]: one instance per connection.
pub struct BackpressureAlgorithmFactory;

impl AbrAlgorithmFactory for BackpressureAlgorithmFactory {
  fn id(&self) -> u64 {
    BACKPRESSURE_ALGORITHM_ID
  }

  fn name(&self) -> &'static str {
    "backpressure"
  }

  fn create(&self) -> Box<dyn AbrAlgorithm> {
    Box::new(BackpressureAlgorithm::new())
  }
}

/// Walks one tier index up and down in response to stream backpressure.
pub struct BackpressureAlgorithm {
  state: TierState,
}

impl BackpressureAlgorithm {
  /// A selector that starts at the lowest tier, so a connection has to earn
  /// its way up.
  pub fn new() -> Self {
    Self {
      state: TierState::default(),
    }
  }
}

impl Default for BackpressureAlgorithm {
  fn default() -> Self {
    Self::new()
  }
}

impl AbrAlgorithm for BackpressureAlgorithm {
  fn decide(&mut self, input: &AbrInput) -> Selection {
    let observation = Observation::new(
      input.active_stream_depth(),
      input.stream_timeouts_since_last_decision,
    );

    match self.state.observe(observation) {
      Event::Held => {}
      Event::CooldownExit => tracing::debug!(
        client_id = input.client_id,
        group_id = input.group_id,
        depth = observation.depth,
        "SSTS backpressure: depth back at target, leaving cooldown"
      ),
      Event::Upshift { from, to } => tracing::debug!(
        client_id = input.client_id,
        group_id = input.group_id,
        from,
        to,
        "SSTS backpressure: UPSHIFT"
      ),
      Event::Downshift { from, to, cause } => tracing::debug!(
        client_id = input.client_id,
        group_id = input.group_id,
        from,
        to,
        depth = observation.depth,
        ?cause,
        "SSTS backpressure: DOWNSHIFT"
      ),
    }

    // The index is shared by the connection's sets, so it cannot be higher
    // than the longest ladder among them; without the clamp a downshift would
    // change nothing while the index sits above every ladder.
    let max_index = input
      .sets
      .iter()
      .map(|set| set.members.len())
      .max()
      .unwrap_or(1)
      .saturating_sub(1);
    self.state.clamp_to(max_index);

    input
      .sets
      .iter()
      .map(|set| {
        let selected = if set.active && !set.members.is_empty() {
          set
            .members
            .get(self.state.tier().min(set.members.len() - 1))
            .map(|(_, track_id)| *track_id)
        } else {
          None
        };
        (set.id, selected)
      })
      .collect()
  }
}
