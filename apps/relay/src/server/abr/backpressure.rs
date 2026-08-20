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

//! Algorithm 1: a backpressure-driven tier selector.
//!
//! Instead of allocating from a bandwidth estimate, this algorithm treats
//! the number of open forwarding streams of the client's switching sets (the
//! "depth"), plus streams that timed out since the last decision, as a
//! congestion signal and walks a single tier index up and down:
//!
//! - while the depth stays at or below `DEPTH_TARGET` for
//!   `UPSHIFT_GOP_STREAK` consecutive groups, the tier is raised by one;
//! - when the depth reaches `DOWNSHIFT_DEPTH`, the tier is lowered by one
//!   and a cooldown starts; during cooldown a stream timeout on the current
//!   tier, or a rising depth for `COOLDOWN_HIGH_STREAK` consecutive groups,
//!   lowers the tier again, and the cooldown ends once the depth is back at
//!   `DEPTH_TARGET`.
//!
//! Every active set forwards the member at the current tier (clamped to its
//! length), so the algorithm always forwards *something* — the lowest tier
//! when the index is zero.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tracing::debug;

use super::{AbrAlgorithm, MOQTClient, SetSnapshot};

/// Depth threshold — any depth above this is congested.
const DEPTH_TARGET: u64 = 1;

/// Immediately downshift if depth hits this.
const DOWNSHIFT_DEPTH: u64 = 2;

/// How many consecutive groups with depth ≤ DEPTH_TARGET before upshifting.
const UPSHIFT_GOP_STREAK: usize = 5;

/// How many consecutive groups in cooldown must show a rising depth before
/// we downshift again. Prevents reacting to a single transient spike.
const COOLDOWN_HIGH_STREAK: usize = 2;

/// Per-client state machine.
struct ClientState {
  /// Index into the ascending tier ladder (0 = lowest quality).
  current_index: usize,
  clear_streak: usize,
  /// Depth at the last downshift; the reference while in cooldown.
  post_downshift_depth: Option<u64>,
  cooldown_high_streak: usize,
  timeouts_since_last_decision: u64,
  current_tier_timeout: bool,
}

impl ClientState {
  fn new() -> Self {
    Self {
      current_index: 0,
      clear_streak: 0,
      post_downshift_depth: None,
      cooldown_high_streak: 0,
      timeouts_since_last_decision: 0,
      current_tier_timeout: false,
    }
  }
}

impl Default for ClientState {
  fn default() -> Self {
    Self::new()
  }
}

/// Algorithm 1: backpressure (open-stream depth) tier selection.
pub struct BackpressureAlgorithm {
  /// State machine per client connection id.
  clients: Mutex<HashMap<u64, ClientState>>,
}

impl BackpressureAlgorithm {
  pub fn new() -> Self {
    Self {
      clients: Mutex::new(HashMap::new()),
    }
  }
}

impl AbrAlgorithm for BackpressureAlgorithm {
  fn id(&self) -> u64 {
    1
  }

  fn on_stream_timeout(&self, client_id: u64) {
    if let Ok(mut clients) = self.clients.lock() {
      let state = clients.entry(client_id).or_default();
      state.timeouts_since_last_decision += 1;
      state.current_tier_timeout = true;
    }
  }

  fn decide<'a>(
    &'a self,
    client: &'a Arc<MOQTClient>,
    group_id: u64,
    sets: &'a [SetSnapshot],
  ) -> Pin<Box<dyn Future<Output = HashMap<u64, Option<u64>>> + Send + 'a>> {
    Box::pin(async move {
      let client_id = client.connection_id as u64;

      // Depth: open forwarding streams of this algorithm's active sets,
      // plus streams that timed out since the last decision.
      let live_depth: u64 = {
        let counters = client.active_streams_per_set.read().await;
        sets
          .iter()
          .filter(|s| s.active)
          .map(|s| {
            counters
              .get(&s.id)
              .map(|c| c.load(Ordering::SeqCst))
              .unwrap_or(0)
          })
          .sum()
      };
      let mut clients = self.clients.lock().unwrap();
      let state = clients.entry(client_id).or_default();
      let depth = live_depth + state.timeouts_since_last_decision;
      state.timeouts_since_last_decision = 0;

      if depth <= DEPTH_TARGET {
        if state.post_downshift_depth.is_some() {
          debug!(
            client_id,
            "SSTS backpressure: depth back to {DEPTH_TARGET}, exiting downshift cooldown"
          );
          state.post_downshift_depth = None;
          state.cooldown_high_streak = 0;
          state.current_tier_timeout = false;
        }
        state.clear_streak += 1;
        if state.clear_streak >= UPSHIFT_GOP_STREAK {
          let candidate = state.current_index + 1;
          debug!(
            client_id,
            from = state.current_index,
            to = candidate,
            streak = state.clear_streak,
            "SSTS backpressure: UPSHIFT"
          );
          state.current_index = candidate;
          state.clear_streak = 0;
        }
      } else if let Some(prev_depth) = state.post_downshift_depth {
        state.clear_streak = 0;
        if state.current_tier_timeout && state.current_index > 0 {
          state.current_tier_timeout = false;
          let target = state.current_index - 1;
          debug!(
            client_id,
            from = state.current_index,
            to = target,
            "SSTS backpressure: DOWNSHIFT (cooldown: timeout on current tier)"
          );
          state.current_index = target;
          state.post_downshift_depth = Some(depth);
          state.cooldown_high_streak = 0;
        } else if depth > prev_depth {
          state.cooldown_high_streak += 1;
          if state.cooldown_high_streak >= COOLDOWN_HIGH_STREAK && state.current_index > 0 {
            let target = state.current_index - 1;
            debug!(
              client_id,
              from = state.current_index,
              to = target,
              depth,
              ref_depth = prev_depth,
              streak = state.cooldown_high_streak,
              "SSTS backpressure: DOWNSHIFT (cooldown: rising depth)"
            );
            state.current_index = target;
            state.post_downshift_depth = Some(depth);
            state.cooldown_high_streak = 0;
          } else {
            state.post_downshift_depth = Some(depth);
          }
        } else {
          state.cooldown_high_streak = 0;
          state.post_downshift_depth = Some(depth);
        }
      } else {
        state.clear_streak = 0;
        if depth >= DOWNSHIFT_DEPTH && state.current_index > 0 {
          let target = state.current_index - 1;
          debug!(
            client_id,
            from = state.current_index,
            to = target,
            depth,
            "SSTS backpressure: DOWNSHIFT"
          );
          state.current_index = target;
          state.post_downshift_depth = Some(depth);
        }
      }

      let max_index = sets
        .iter()
        .map(|s| s.members.len())
        .max()
        .unwrap_or(1)
        .saturating_sub(1);
      state.current_index = state.current_index.min(max_index);

      debug!(
        client_id,
        group_id,
        depth,
        clear_streak = state.clear_streak,
        in_cooldown = state.post_downshift_depth.is_some(),
        current_tier = state.current_index,
        "SSTS backpressure: allocation"
      );

      // Every active set forwards the member at the current tier (clamped).
      let mut decisions: HashMap<u64, Option<u64>> = HashMap::new();
      for set in sets {
        if !set.active || set.members.is_empty() {
          decisions.insert(set.id, None);
          continue;
        }
        let tier = state.current_index.min(set.members.len() - 1);
        decisions.insert(set.id, Some(set.members[tier].1));
      }
      decisions
    })
  }
}
