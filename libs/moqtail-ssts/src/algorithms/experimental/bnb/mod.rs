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

//! Budget-negotiated backpressure: each set reacts to how much of its own
//! subscriber's latency budget its backlog has used.
//!
//! The depth algorithms ask how many streams are open. That is a quantity
//! without a unit: three open streams on a connection carrying four-second
//! groups is an empty pipeline, and three on one carrying 200 ms groups is a
//! collapse. This algorithm puts a unit on it by asking the subscriber. The
//! SWITCHING_SET_ASSIGNMENT extension carries the subscriber's reorder buffer,
//! which is exactly how long it is willing to wait for a group, and the set's
//! backlog is the age of its oldest open stream beyond one group period.
//! Backlog over the buffer means a discard is coming; backlog under a fifth of
//! it means there is room to try a higher rung.
//!
//! Each set has its own rung index, its own streaks and its own budget, so
//! unlike the depth algorithms a subscriber with several switching sets sees
//! each of them move on its own evidence. The rules, per decision:
//!
//! * backlog over `danger_factor` of the buffer for `downshift_danger_streak`
//!   groups: drop one rung;
//! * backlog in between, and the queue growing since the last group: drop one;
//! * backlog under `safe_factor`, the queue not growing, and no timeout veto,
//!   for `upshift_safe_streak` groups: climb one;
//! * timeouts on this set are a separate channel that vetoes climbs and, once
//!   sustained, force a drop even when the queue looks drained.
//!
//! After any change the rung holds for `drain_window_ms`, because the samples
//! taken just after a move measure the old rung's streams finishing, not the
//! new state.
//!
//! See the README in this directory for what this does badly.

mod state;

use std::collections::HashMap;

use crate::AbrAlgorithm;
use crate::AbrAlgorithmFactory;
use crate::AbrInput;
use crate::Selection;
use crate::Tuning;

#[cfg(test)]
mod tests;

use state::Action;
use state::BnbParams;
use state::SetState;
use state::Signals;

/// The third id in the range the specification leaves to implementations.
pub const BNB_ALGORITHM_ID: u64 = crate::registry::PRIVATE_ALGORITHM_ID_BASE + 2;

/// Factory for [`BnbAlgorithm`]: one instance per connection, tuned by the host.
pub struct BnbAlgorithmFactory;

impl AbrAlgorithmFactory for BnbAlgorithmFactory {
  fn id(&self) -> u64 {
    BNB_ALGORITHM_ID
  }

  fn name(&self) -> &'static str {
    "bnb"
  }

  fn create(&self, tuning: &Tuning) -> Box<dyn AbrAlgorithm> {
    Box::new(BnbAlgorithm::with_params(BnbParams::from_tuning(tuning)))
  }
}

/// Budget-based rung selection, one rung index per switching set.
pub struct BnbAlgorithm {
  params: BnbParams,
  /// Per-set state, keyed by switching set id. Entries are dropped with their
  /// set, so a long-lived connection does not accumulate state for sets that
  /// came and went.
  sets: HashMap<u64, SetState>,
}

impl BnbAlgorithm {
  /// The algorithm with its default thresholds.
  pub fn new() -> Self {
    Self::with_params(BnbParams::default())
  }

  /// The algorithm with thresholds from the host, which is how an experiment
  /// runs without a rebuild.
  pub fn with_params(params: BnbParams) -> Self {
    Self {
      params,
      sets: HashMap::new(),
    }
  }

  /// The rung a set is on, for tests and diagnostics.
  pub fn tier(&self, set_id: u64, max_index: usize) -> usize {
    self
      .sets
      .get(&set_id)
      .map(|s| s.tier(max_index))
      .unwrap_or(0)
  }
}

impl Default for BnbAlgorithm {
  fn default() -> Self {
    Self::new()
  }
}

impl AbrAlgorithm for BnbAlgorithm {
  fn decide(&mut self, input: &AbrInput) -> Selection {
    let mut selection = Selection::with_capacity(input.sets.len());

    for set in input.sets {
      let Some(max_index) = set.members.len().checked_sub(1) else {
        // No members: nothing to forward, and no state to keep for it.
        selection.insert(set.id, None);
        continue;
      };
      if !set.active {
        // No subscriber: forward nothing, and do not let the rung drift on
        // evidence nobody is consuming.
        selection.insert(set.id, None);
        continue;
      }

      let state = self.sets.entry(set.id).or_default();
      let signals = Signals {
        depth: input.open_streams(set.id),
        // A healthy group's stream lives one group span, so only the age beyond
        // it is accumulation. Until the host has measured a span, nothing is
        // backlog, and the algorithm climbs on its streaks alone.
        backlog_ms: input
          .oldest_stream_age_ms(set.id)
          .saturating_sub(input.group_span_ms(set.id)),
        budget_ms: set.subscriber_buffer_ms,
        timeouts: input.stream_timeouts(set.id),
      };

      let from = state.tier(max_index);
      let action = state.step(max_index, &signals, &self.params, input.now_ms);
      let to = state.tier(max_index);

      log(
        input,
        set.id,
        action,
        from,
        to,
        &signals,
        state.timeout_streak_for_log(),
      );

      selection.insert(set.id, set.members[to].1);
    }

    // A set that is gone takes its streaks with it: a publisher that reuses the
    // id starts from the same place a fresh connection would.
    self
      .sets
      .retain(|set_id, _| input.sets.iter().any(|set| &set.id == set_id));

    selection
  }
}

/// One line per decision that moved something, and one per drain-window hold.
fn log(
  input: &AbrInput,
  set_id: u64,
  action: Action,
  from: usize,
  to: usize,
  signals: &Signals,
  timeout_streak: usize,
) {
  match action {
    Action::Hold => {}
    Action::HoldInDrain => tracing::debug!(
      client_id = input.client_id,
      group_id = input.group_id,
      set_id,
      tier = to,
      backlog_ms = signals.backlog_ms,
      budget_ms = signals.budget_ms,
      "SSTS bnb: HOLD (drain window)"
    ),
    _ => tracing::debug!(
      client_id = input.client_id,
      group_id = input.group_id,
      set_id,
      from,
      to,
      backlog_ms = signals.backlog_ms,
      budget_ms = signals.budget_ms,
      depth = signals.depth,
      timeout_streak,
      ?action,
      "SSTS bnb: tier change"
    ),
  }
}
