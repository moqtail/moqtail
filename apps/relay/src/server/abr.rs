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

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::debug;
use tracing::info;

use crate::server::client::AbrMessage;
use crate::server::client::MOQTClient;

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

const TICK_MS: u64 = 100;

/// Depth threshold — any depth above this is congested.
const DEPTH_TARGET: u64 = 1;

/// Immediately downshift if depth hits this.
const DOWNSHIFT_DEPTH: u64 = 2;

/// How many consecutive GOPs with depth ≤ DEPTH_TARGET before upshifting.
const UPSHIFT_GOP_STREAK: usize = 5;

/// How many consecutive GOPs in cooldown must show depth > reference
/// before we downshift again. Prevents reacting to a single transient spike.
const COOLDOWN_HIGH_STREAK: usize = 2;

const DISCARD_TIMEOUT_MS: u64 = 1600;

// ─────────────────────────────────────────────────────────────────────────────
// Persistent state
// ─────────────────────────────────────────────────────────────────────────────

struct AbrState {
  clear_streak: usize,
  post_downshift_depth: Option<u64>,
  cooldown_high_streak: usize,
  timeouts_since_last_decision: u64,
  current_tier_timeout: bool,
}

impl AbrState {
  fn new() -> Self {
    Self {
      clear_streak: 0,
      post_downshift_depth: None,
      cooldown_high_streak: 0,
      timeouts_since_last_decision: 0,
      current_tier_timeout: false,
    }
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Main controller task
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn start_abr_controller(client: Arc<MOQTClient>) {
  let client_id = client.connection_id as u64;
  tokio::spawn(async move {
    let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started once");

    let mut current_index = 0usize;
    let mut state = AbrState::new();
    let mut decided_groups = std::collections::HashSet::new();

    let mut tick = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

    client
      .discard_timeout_ms
      .store(DISCARD_TIMEOUT_MS, Ordering::Relaxed);

    loop {
      tokio::select! {
          msg = abr_rx.recv() => {
              let msg = match msg { Some(m) => m, None => break };

              match msg {
                  AbrMessage::NewGroup(group_id) => {
                      let already_decided_and_cached = {
                    let decisions = client.group_decisions.read().await;
                        decided_groups.contains(&group_id) && decisions.contains_key(&group_id)
                    };

                    if already_decided_and_cached {
                        // We already decided and it's still cached.
                        // Still notify the subscriber so it can wake up, find the decision, and proceed.
                        client.decision_notify.notify_waiters();
                        continue;
                    }

                      // Otherwise, proceed with making the decision...
                      decided_groups.insert(group_id);

                      // ── 2. Build Dynamic Bitrate Ladder from Switching Sets ─────────────
                      // (moved before depth calculation so we can sum per-set counters)
                      let mut active_sets = Vec::new();
                      {
                          let manager = client.switching_sets.read().await;
                          for set in manager.sets.values() {
                              if set.active && !set.members.is_empty() {
                                  let mut members = set.members.clone();
                                  // Sort members by throughput ascending (lowest quality = index 0)
                                  members.sort_by_key(|m| m.throughput_threshold_kbps);
                                  active_sets.push((set.id, members));
                              }
                          }
                      }

                      if active_sets.is_empty() {
                          client.decision_notify.notify_waiters();
                          continue;
                      }

                      // Calculate live_depth as sum of open streams in active switching sets only
                      let live_depth: u64 = {
                          let counters = client.active_streams_per_set.read().await;
                          active_sets.iter().map(|(set_id, _)| {
                              counters.get(set_id)
                                  .map(|c| c.load(Ordering::SeqCst) as u64)
                                  .unwrap_or(0)
                          }).sum()
                      };
                      let depth = live_depth + state.timeouts_since_last_decision;
                      state.timeouts_since_last_decision = 0;

                      // ── 1. Backpressure Logic ──────────────
                      if depth <= DEPTH_TARGET {
                          if state.post_downshift_depth.is_some() {
                              debug!(client_id = %client_id, "ABR: depth back to {DEPTH_TARGET}, exiting downshift cooldown");
                              state.post_downshift_depth = None;
                              state.cooldown_high_streak = 0;
                              state.current_tier_timeout = false;
                          }
                          state.clear_streak += 1;
                          if state.clear_streak >= UPSHIFT_GOP_STREAK {
                              let candidate = current_index + 1;
                              debug!(
                                  client_id = %client_id,
                                  "ABR: UPSHIFT {} → {} ({} consecutive clear GOPs)",
                                  current_index, candidate, state.clear_streak
                              );
                              current_index = candidate;
                              state.clear_streak = 0;
                          }
                      } else if let Some(prev_depth) = state.post_downshift_depth {
                          state.clear_streak = 0;
                          if state.current_tier_timeout && current_index > 0 {
                              state.current_tier_timeout = false;
                              let target = current_index - 1;
                              debug!(
                                  client_id = %client_id,
                                  "ABR: DOWNSHIFT {} → {} (cooldown: timeout on current tier)",
                                  current_index, target,
                              );
                              current_index = target;
                              state.post_downshift_depth = Some(depth);
                              state.cooldown_high_streak = 0;
                          } else if depth > prev_depth {
                              state.cooldown_high_streak += 1;
                              if state.cooldown_high_streak >= COOLDOWN_HIGH_STREAK && current_index > 0 {
                                  let target = current_index - 1;
                                  debug!(
                                      client_id = %client_id,
                                      "ABR: DOWNSHIFT {} → {} (cooldown: depth {} > ref {} for {} GOPs)",
                                      current_index, target, depth, prev_depth, state.cooldown_high_streak,
                                  );
                                  current_index = target;
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
                          if depth >= DOWNSHIFT_DEPTH && current_index > 0 {
                              let target = current_index - 1;
                              debug!(
                                  client_id = %client_id,
                                  "ABR: DOWNSHIFT {} → {} (depth={} ≥ {})",
                                  current_index, target, depth, DOWNSHIFT_DEPTH
                              );
                              current_index = target;
                              state.post_downshift_depth = Some(depth);
                          }
                      }

                      let max_index = active_sets.iter().map(|(_, m)| m.len()).max().unwrap_or(1) - 1;
                      current_index = current_index.min(max_index);

                      // ── 3. Make Decisions for Each Set ───────────────────────────
                      let mut decisions: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
                      for (set_id, members) in &active_sets {
                          let tier = current_index.min(members.len() - 1);
                          let selected_relay_id = members[tier].relay_track_id;
                          decisions.insert(*set_id, selected_relay_id);
                      }

                      info!(
                          client_id = %client_id,
                          "ABR: group={group_id} depth={depth} clear_streak={} cooldown={} current_tier={}",
                          state.clear_streak, state.post_downshift_depth.is_some(), current_index
                      );

                      // ── 4. Apply Decisions ───────────────────────────────────────
                      {
                          let mut group_decisions = client.group_decisions.write().await;
                          for g in group_id.saturating_sub(3)..=group_id {
                              group_decisions.insert(g, decisions.clone());
                              client.decision_notify.notify_waiters();
                          }
                          group_decisions.retain(|&k, _| k >= group_id.saturating_sub(5));
                      }

                  }

                  AbrMessage::StreamTimeout { group_id} => {
                      // Collect per-set depths, drop the lock before logging
                      let per_set_depths: Vec<(u64, u64)> = {
                          let counters = client.active_streams_per_set.read().await;
                          counters.iter().map(|(set_id, c)| (*set_id, c.load(Ordering::SeqCst) as u64)).collect()
                      };
                      let total_depth: u64 = per_set_depths.iter().map(|(_, d)| d).sum();
                      info!(
                          client_id = %client_id,
                          "ABR: stream timeout — group={group_id} total_depth={total_depth} — counting toward adjusted depth"
                      );
                      state.timeouts_since_last_decision += 1;
                      state.current_tier_timeout = true;
                  }
              }
          }

          _ = tick.tick() => {
              if decided_groups.is_empty() { continue; }

              // Collect per-set depths, drop the lock before logging
              let per_set_depths: Vec<(u64, u64)> = {
                  let counters = client.active_streams_per_set.read().await;
                  counters.iter().map(|(set_id, c)| (*set_id, c.load(Ordering::SeqCst) as u64)).collect()
              };

              // Also log aggregate state
              let total_depth: u64 = per_set_depths.iter().map(|(_, d)| d).sum();
              tracing::debug!(
                  client_id = %client_id,
                  active_depth = total_depth,
                  current_tier = current_index,
                  clear_streak = state.clear_streak,
                  in_cooldown = state.post_downshift_depth.is_some(),
              );
          }

          _ = client.connection.closed() => {
              tracing::info!(client_id = %client_id, "ABR controller shutting down: connection physically closed");
              break;
          }
      }
    }
  });
}
