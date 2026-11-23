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

//! ASD (Active Stream Depth) tier selection: how many streams are still in
//! flight, rather than how much bandwidth is estimated to be available.
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

pub(crate) mod state;

use crate::{AbrAlgorithm, AbrAlgorithmFactory, AbrInput, DecisionReason, Selection, Tuning};

#[cfg(test)]
mod tests;

use state::{Event, Observation, TierState};

/// MOQtail's ASD (Active Stream Depth). This is not a registered id: it is the
/// first value in the range the specification leaves to implementations, so
/// experimenting with it cannot collide with a future allocation. See
/// [`crate::registry::PRIVATE_ALGORITHM_ID_BASE`].
pub const ASD_ALGORITHM_ID: u64 = crate::registry::PRIVATE_ALGORITHM_ID_BASE;

/// Factory for [`AsdAlgorithm`]: one instance per connection.
pub struct AsdAlgorithmFactory;

impl AbrAlgorithmFactory for AsdAlgorithmFactory {
  fn id(&self) -> u64 {
    ASD_ALGORITHM_ID
  }

  fn name(&self) -> &'static str {
    "asd"
  }

  fn create(&self, _tuning: &Tuning) -> Box<dyn AbrAlgorithm> {
    Box::new(AsdAlgorithm::new())
  }
}

/// Walks one tier index up and down in response to stream backpressure.
pub struct AsdAlgorithm {
  state: TierState,
  /// Discards reported by decisions that are not the group boundary.
  ///
  /// The paper reads `discards_since_last_decision` once, on the new-group
  /// boundary. The host also re-evaluates on a stream timeout, and that
  /// decision clears the host's counter; the discards are kept here until the
  /// boundary that is supposed to see them.
  pending_timeouts: u64,
}

impl AsdAlgorithm {
  /// A selector that starts at the lowest tier, so a connection has to earn
  /// its way up.
  pub fn new() -> Self {
    Self {
      state: TierState::default(),
      pending_timeouts: 0,
    }
  }
}

impl Default for AsdAlgorithm {
  fn default() -> Self {
    Self::new()
  }
}

impl AbrAlgorithm for AsdAlgorithm {
  fn decide(&mut self, input: &AbrInput) -> Selection {
    // The paper's algorithm decides on the new-group boundary: one
    // observation per group, with `d = open_streams + discards_since_last`.
    // The host also calls this on a stream timeout and on the object-paced
    // sample. Those decisions serve the tier in hand, but they must not walk
    // the state machine: a timeout decision would otherwise apply the same
    // group's discards once per reset, and a burst inside one group would
    // fall several tiers for one group's evidence. The discards are not lost;
    // they are accumulated and read at the next boundary, which is where the
    // paper reads them.
    self.pending_timeouts += input.total_stream_timeouts_since_last_decision();

    if input.reason == DecisionReason::GroupBoundary {
      let observation = Observation::new(
        input.active_stream_depth(),
        std::mem::take(&mut self.pending_timeouts),
      );

      match self.state.observe(observation) {
        Event::Held => {}
        Event::CooldownExit => tracing::debug!(
          client_id = input.client_id,
          group_id = input.group_id,
          depth = observation.depth,
          "SSTS asd: depth back at target, leaving cooldown"
        ),
        Event::Upshift { from, to } => tracing::debug!(
          client_id = input.client_id,
          group_id = input.group_id,
          from,
          to,
          "SSTS asd: UPSHIFT"
        ),
        Event::Downshift { from, to, cause } => tracing::debug!(
          client_id = input.client_id,
          group_id = input.group_id,
          from,
          to,
          depth = observation.depth,
          ?cause,
          "SSTS asd: DOWNSHIFT"
        ),
      }
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

    crate::algorithms::serve_tier(input, self.state.tier())
  }
}
