use moqtail::model::data::full_track_name::FullTrackName;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tracing::{info, warn};

use crate::server::client::AbrMessage;
use crate::server::client::MOQTClient;
use crate::server::track_manager::TrackManager;

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Tick period for the ABR loop.
const TICK_MS: u64 = 100;

const NUM_TIERS: usize = 4; // 360p, 480p, 720p, 1080p

/// Fixed stream discard timeout in milliseconds.
/// A group that has not finished sending within this budget is cancelled via
/// RESET_STREAM and the backlog counter is decremented.
/// Set to roughly one GOP at low latency (600 ms is already in the code).
const DISCARD_TIMEOUT_MS: u64 = 600;

// ─────────────────────────────────────────────────────────────────────────────
// Buffer thresholds (measured in concurrent active send tasks / groups)
// ─────────────────────────────────────────────────────────────────────────────
//
// The relay holds at most ~2 GOPs in-flight before it starts shedding quality.
// These levels map the send-buffer depth to a quality ceiling:
//
//   depth 0-1  → full quality available (up to max tier)
//   depth 2    → cap at tier 2  (720p)
//   depth 3    → cap at tier 1  (480p)
//   depth 4+   → cap at tier 0  (360p, floor)
//
// This is the BOLA-style insight: buffer occupancy is the ground truth about
// whether the network can sustain the current bitrate.  No RTT estimation,
// no app-limited heuristics — just: is the send buffer growing?
//
const BUFFER_LEVEL_TIER_3: u64 = 1; // depth ≤ this → tier 3 (1080p) allowed
const BUFFER_LEVEL_TIER_2: u64 = 2; // depth ≤ this → tier 2 (720p) allowed
const BUFFER_LEVEL_TIER_1: u64 = 3; // depth ≤ this → tier 1 (480p) allowed
                                     // depth > 3    → tier 0 (360p) only

/// After a timeout we penalise that tier so we don't immediately retry it.
const TIER_PENALTY_SECS: u64 = 15;

// ─────────────────────────────────────────────────────────────────────────────
// Tier helper (unchanged — keeps parity with the rest of the codebase)
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn tier_index_for_track(track_name: &FullTrackName) -> usize {
    let name = track_name.to_string();
    if name.contains("360p") {
        0
    } else if name.contains("480p") {
        1
    } else if name.contains("720p") {
        2
    } else if name.contains("1080p") {
        3
    } else {
        0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Persistent state
// ─────────────────────────────────────────────────────────────────────────────

struct AbrState {
    tier_failure_times: [Option<Instant>; NUM_TIERS],
}

impl AbrState {
    fn new() -> Self {
        Self {
            tier_failure_times: [None; NUM_TIERS],
        }
    }

    fn tier_is_penalised(&self, tier: usize) -> bool {
        if tier >= NUM_TIERS {
            return false;
        }
        match self.tier_failure_times[tier] {
            Some(t) => t.elapsed().as_secs() < TIER_PENALTY_SECS,
            None => false,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Core decision function
// ─────────────────────────────────────────────────────────────────────────────

/// Maps the current send-buffer depth to the highest tier index we are
/// allowed to serve.  Lower depth → higher ceiling → better quality.
///
/// This is the entire ABR algorithm.  The buffer depth IS the network signal.
fn buffer_quality_ceiling(depth: u64) -> usize {
    if depth <= BUFFER_LEVEL_TIER_3 {
        3
    } else if depth <= BUFFER_LEVEL_TIER_2 {
        2
    } else if depth <= BUFFER_LEVEL_TIER_1 {
        1
    } else {
        0
    }
}

/// Select the best tier that is (a) ≤ ceiling and (b) not currently penalised.
/// Falls back toward tier 0 if all candidates are penalised.
fn select_tier(ceiling: usize, state: &AbrState) -> usize {
    let mut candidate = ceiling;
    loop {
        if !state.tier_is_penalised(candidate) {
            return candidate;
        }
        if candidate == 0 {
            return 0; // always serve tier 0, even if penalised
        }
        candidate -= 1;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn sorted_tracks(track_manager: &TrackManager) -> Vec<FullTrackName> {
    let mut tracks: Vec<FullTrackName> = track_manager
        .track_aliases
        .read()
        .await
        .values()
        .cloned()
        .collect();
    tracks.sort_by_key(|t| {
        let name = t.to_string();
        if name.contains("360p") {
            0
        } else if name.contains("480p") {
            1
        } else if name.contains("720p") {
            2
        } else if name.contains("1080p") {
            3
        } else {
            4
        }
    });
    tracks
}

// ─────────────────────────────────────────────────────────────────────────────
// Main controller task
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn start_abr_controller(client: Arc<MOQTClient>, track_manager: Arc<TrackManager>) {
    tokio::spawn(async move {
        let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started once");

        let mut current_index = 0usize; // start at 360p; upshift is earned
        let mut state = AbrState::new();
        let mut last_decided_group = 0u64;

        let mut tick = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

        // Stamp the fixed timeout into the shared atomic immediately so that
        // write_stream_object and close_stream use the right value from the start.
        client
            .discard_timeout_ms
            .store(DISCARD_TIMEOUT_MS, Ordering::Relaxed);

        info!(
            "ABR controller started: pure-backpressure mode, \
             discard_timeout={DISCARD_TIMEOUT_MS}ms, \
             tier_penalty={TIER_PENALTY_SECS}s."
        );

        loop {
            tokio::select! {

                // ── Branch 1: incoming message from publisher / stream layer ──
                msg = abr_rx.recv() => {
                    let msg = match msg {
                        Some(m) => m,
                        None => break,
                    };

                    match msg {
                        // ── New group: apply current quality decision ─────────
                        AbrMessage::NewGroup(group_id) => {
                            if group_id <= last_decided_group {
                                continue;
                            }
                            last_decided_group = group_id;

                            // Read the live backlog depth right now.
                            let depth =
                                client.active_send_tasks.load(Ordering::SeqCst) as u64;
                            let ceiling = buffer_quality_ceiling(depth);
                            let target_index = select_tier(ceiling, &state);

                            // Log the backpressure state alongside the decision.
                            info!(
                                "ABR: group={group_id} \
                                 send_buffer_depth={depth} \
                                 quality_ceiling={ceiling} \
                                 selected_tier={target_index} \
                                 (current={current_index})"
                            );

                            if target_index != current_index {
                                if target_index < current_index {
                                    warn!(
                                        "ABR: DOWNSHIFT {} → {} (depth={})",
                                        current_index, target_index, depth
                                    );
                                } else {
                                    info!(
                                        "ABR: UPSHIFT {} → {} (depth={})",
                                        current_index, target_index, depth
                                    );
                                }
                                current_index = target_index;
                            }

                            // Stamp the decision so the subscription layer can
                            // pick the right track for this group.
                            let tracks = sorted_tracks(&track_manager).await;
                            if tracks.is_empty() {
                                continue;
                            }
                            let max_index = tracks.len().saturating_sub(1);
                            let safe_index = current_index.min(max_index);
                            let target_track = &tracks[safe_index];
                            let target_alias = {
                                let aliases = track_manager.track_aliases.read().await;
                                aliases
                                    .iter()
                                    .find(|(_, name)| *name == target_track)
                                    .map(|(alias, _)| alias.1)
                            };
                            if let Some(alias) = target_alias {
                                let mut decisions = client.group_decisions.write().await;
                                for g in group_id.saturating_sub(3)..=group_id {
                                    decisions.insert(g, alias);
                                    let _ = client.decision_tx.send(g);
                                }
                                decisions.retain(|&k, _| k >= group_id.saturating_sub(5));
                            }
                            crate::telemetry::log_abr_decision(
                                group_id,
                                &target_track.to_string(),
                            );
                        }

                        // ── Stream timeout: penalise the offending tier ───────
                        AbrMessage::StreamTimeout { group_id, tier_index } => {
                            let depth =
                                client.active_send_tasks.load(Ordering::SeqCst) as u64;
                            warn!(
                                "ABR: stream timeout — group={group_id} \
                                 tier={tier_index} \
                                 send_buffer_depth={depth} \
                                 — penalising tier for {TIER_PENALTY_SECS}s"
                            );
                            if tier_index < state.tier_failure_times.len() {
                                state.tier_failure_times[tier_index] = Some(Instant::now());
                            }
                        }
                    }
                }

                // ── Branch 2: 100 ms tick — telemetry only ────────────────────
                _ = tick.tick() => {
                    if last_decided_group == 0 {
                        continue;
                    }

                    let depth = client.active_send_tasks.load(Ordering::SeqCst) as u64;
                    let ceiling = buffer_quality_ceiling(depth);

                    // Keep the telemetry calls the dashboard already knows about.
                    let smoothed_rtt_ms =
                        client.connection.rtt().as_secs_f64() * 1000.0;
                    let min_rtt_ms =
                        client.connection.min_rtt().as_secs_f64() * 1000.0;
                    let latest_rtt_ms =
                        client.connection.latest_rtt().as_secs_f64() * 1000.0;
                    let cwnd_bytes = client.connection.cwnd();
                    let bw_estimate = client.connection.bw_estimate();
                    let app_limited = client.connection.app_limited();
                    let discard_ms =
                        client.discard_timeout_ms.load(Ordering::Relaxed);

                    crate::telemetry::log_network_stats(
                        smoothed_rtt_ms as u128,
                        min_rtt_ms as u128,
                        latest_rtt_ms as u128,
                        cwnd_bytes,
                        bw_estimate,
                        depth,
                        app_limited,
                        discard_ms,
                    );

                    // ABR-state telemetry: reuse existing call but map our
                    // concepts onto its fields so dashboards keep working.
                    // • "state_label"        → "BUFFER:<depth>"
                    // • queueing_delay_ms    → depth as a pseudo-float (for charting)
                    // • window_app_limited   → ceiling / (NUM_TIERS-1) as a fraction
                    // • backlog_rising       → false (not used by new logic)
                    // • loss_ewma            → 0.0   (not used by new logic)
                    let state_label = format!("BUFFER:{}", depth);
                    crate::telemetry::log_abr_state(
                        &state_label,
                        current_index,
                        depth as f64,
                        ceiling as f64 / (NUM_TIERS - 1) as f64,
                        false,
                        0.0,
                    );
                }
            }
        }
    });
}