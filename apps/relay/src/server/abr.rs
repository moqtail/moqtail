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

const TICK_MS: u64 = 100;

/// Depth threshold — any depth above this is congested.
const DEPTH_TARGET: u64 = 1;

/// Immediately downshift if depth hits this.
const DOWNSHIFT_DEPTH: u64 = 3;

/// How many consecutive GOPs with depth ≤ DEPTH_TARGET before upshifting.
const UPSHIFT_GOP_STREAK: usize = 5;

const NUM_TIERS: usize = 4;
const TIER_PENALTY_SECS: u64 = 1;

/// Hard discard timeout — stream is reset if it hasn't finished by this point.
/// QUIC priorities drain newer groups first; this catches anything truly stale.
const DISCARD_TIMEOUT_MS: u64 = 1100;

// ─────────────────────────────────────────────────────────────────────────────
// Tier helper
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn tier_index_for_track(track_name: &FullTrackName) -> usize {
    let name = track_name.to_string();
    if name.contains("360p")       { 0 }
    else if name.contains("480p")  { 1 }
    else if name.contains("720p")  { 2 }
    else if name.contains("1080p") { 3 }
    else { 0 }
}

// ─────────────────────────────────────────────────────────────────────────────
// Persistent state
// ─────────────────────────────────────────────────────────────────────────────

struct AbrState {
    /// Consecutive GOPs where depth ≤ DEPTH_TARGET on arrival.
    clear_streak:         usize,
    /// Set to the depth reading at the moment we downshift.
    /// While Some, we are in "cooldown" — we ignore the absolute depth and
    /// only downshift again if depth has grown since the last downshift.
    /// Cleared back to None once depth hits DEPTH_TARGET (baseline restored).
    post_downshift_depth: Option<u64>,
    tier_failure_times:   [Option<Instant>; NUM_TIERS],
}

impl AbrState {
    fn new() -> Self {
        Self {
            clear_streak:         0,
            post_downshift_depth: None,
            tier_failure_times:   [None; NUM_TIERS],
        }
    }

    fn tier_is_penalised(&self, tier: usize) -> bool {
        if tier >= NUM_TIERS { return false; }
        match self.tier_failure_times[tier] {
            Some(t) => t.elapsed().as_secs() < TIER_PENALTY_SECS,
            None    => false,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn sorted_tracks(track_manager: &TrackManager) -> Vec<FullTrackName> {
    let mut tracks: Vec<FullTrackName> = track_manager
        .track_aliases.read().await.values().cloned().collect();
    tracks.sort_by_key(|t| {
        let name = t.to_string();
        if name.contains("360p")       { 0 }
        else if name.contains("480p")  { 1 }
        else if name.contains("720p")  { 2 }
        else if name.contains("1080p") { 3 }
        else { 4 }
    });
    tracks
}

// ─────────────────────────────────────────────────────────────────────────────
// Main controller task
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn start_abr_controller(client: Arc<MOQTClient>, track_manager: Arc<TrackManager>) {
    tokio::spawn(async move {
        let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started once");

        let mut current_index      = 0usize;
        let mut state              = AbrState::new();
        let mut last_decided_group = 0u64;

        let mut tick = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

        client.discard_timeout_ms.store(DISCARD_TIMEOUT_MS, Ordering::Relaxed);

        info!(
            "ABR controller started: GOP-streak backpressure mode, \
             immediate downshift at depth≥{DOWNSHIFT_DEPTH}, \
             upshift after {UPSHIFT_GOP_STREAK} clear GOPs (depth≤{DEPTH_TARGET}), \
             discard_timeout={DISCARD_TIMEOUT_MS}ms, tier_penalty={TIER_PENALTY_SECS}s."
        );

        loop {
            tokio::select! {

                // ── Branch 1: incoming message from publisher / stream layer ──
                msg = abr_rx.recv() => {
                    let msg = match msg { Some(m) => m, None => break };

                    match msg {
                        AbrMessage::NewGroup(group_id) => {
                            if group_id <= last_decided_group { continue; }
                            last_decided_group = group_id;

                            let depth = client.active_send_tasks.load(Ordering::SeqCst) as u64;

                            // ── Baseline restored: exit cooldown ──────────────
                            // Old groups have drained, we are back to steady state.
                            // Re-enable normal absolute-depth downshift logic.
                            if depth <= DEPTH_TARGET {
                                if state.post_downshift_depth.is_some() {
                                    info!("ABR: depth back to {DEPTH_TARGET}, exiting downshift cooldown");
                                    state.post_downshift_depth = None;
                                }
                                state.clear_streak += 1;
                                if state.clear_streak >= UPSHIFT_GOP_STREAK
                                    && current_index < (NUM_TIERS - 1)
                                    && !state.tier_is_penalised(current_index + 1)
                                {
                                    let candidate = current_index + 1;
                                    info!(
                                        "ABR: UPSHIFT {} → {} ({} consecutive clear GOPs)",
                                        current_index, candidate, state.clear_streak
                                    );
                                    current_index = candidate;
                                    state.clear_streak = 0;
                                }
                            } else if let Some(prev_depth) = state.post_downshift_depth {
                                // ── In cooldown: only react to depth growing ───
                                // The old groups are still draining — ignore the
                                // absolute value and only downshift again if depth
                                // has actually increased since the last downshift.
                                state.clear_streak = 0;
                                if depth > prev_depth && current_index > 0 {
                                    let target = current_index - 1;
                                    if current_index < NUM_TIERS {
                                        state.tier_failure_times[current_index] = Some(Instant::now());
                                    }
                                    warn!(
                                        "ABR: DOWNSHIFT {} → {} (cooldown: depth grew {} → {})",
                                        current_index, target, prev_depth, depth
                                    );
                                    current_index = target;
                                    state.post_downshift_depth = Some(depth);
                                } else {
                                    // Depth is holding or shrinking — old groups draining,
                                    // update the reference so we only react to new growth.
                                    state.post_downshift_depth = Some(depth);
                                    info!(
                                        "ABR: cooldown — depth={depth} (was {prev_depth}), holding tier={}",
                                        current_index
                                    );
                                }
                            } else {
                                // ── Normal mode: immediate downshift at threshold ──
                                state.clear_streak = 0;
                                if depth >= DOWNSHIFT_DEPTH && current_index > 0 {
                                    let target = current_index - 1;
                                    if current_index < NUM_TIERS {
                                        state.tier_failure_times[current_index] = Some(Instant::now());
                                    }
                                    warn!(
                                        "ABR: DOWNSHIFT {} → {} (depth={} ≥ {})",
                                        current_index, target, depth, DOWNSHIFT_DEPTH
                                    );
                                    current_index = target;
                                    // Enter cooldown: record depth at the moment of downshift.
                                    state.post_downshift_depth = Some(depth);
                                }
                            }

                            info!(
                                "ABR: group={group_id} depth={depth} \
                                 clear_streak={} cooldown={} current_tier={}",
                                state.clear_streak,
                                state.post_downshift_depth.is_some(),
                                current_index
                            );

                            let tracks = sorted_tracks(&track_manager).await;
                            if tracks.is_empty() { continue; }
                            let max_index    = tracks.len().saturating_sub(1);
                            let target_track = &tracks[current_index.min(max_index)];
                            let target_relay_id = if let Some(track_arc) =
                                track_manager.get_track(target_track).await
                            {
                                let track = track_arc.read().await;
                                Some(track.relay_track_id)
                            } else {
                                None
                            };
                            if let Some(relay_id) = target_relay_id {
                                let mut decisions = client.group_decisions.write().await;
                                for g in group_id.saturating_sub(3)..=group_id {
                                    decisions.insert(g, relay_id);
                                    let _ = client.decision_tx.send(g);
                                }
                                decisions.retain(|&k, _| k >= group_id.saturating_sub(5));
                            }
                            crate::telemetry::log_abr_decision(group_id, &target_track.to_string());
                        }

                        AbrMessage::StreamTimeout { group_id, tier_index } => {
                            let depth = client.active_send_tasks.load(Ordering::SeqCst) as u64;
                            warn!(
                                "ABR: stream timeout — group={group_id} tier={tier_index} \
                                 depth={depth} — penalising tier for {TIER_PENALTY_SECS}s"
                            );
                            if tier_index < state.tier_failure_times.len() {
                                state.tier_failure_times[tier_index] = Some(Instant::now());
                            }
                        }
                    }
                }

                // ── Branch 2: 100 ms tick — telemetry only ────────────────────
                _ = tick.tick() => {
                    if last_decided_group == 0 { continue; }

                    let depth           = client.active_send_tasks.load(Ordering::SeqCst) as u64;
                    let smoothed_rtt_ms = client.connection.rtt().as_secs_f64() * 1000.0;
                    let min_rtt_ms      = client.connection.min_rtt().as_secs_f64() * 1000.0;
                    let latest_rtt_ms   = client.connection.latest_rtt().as_secs_f64() * 1000.0;
                    let cwnd_bytes      = client.connection.cwnd();
                    let bw_estimate     = client.connection.bw_estimate();
                    let app_limited     = client.connection.app_limited();
                    let discard_ms      = client.discard_timeout_ms.load(Ordering::Relaxed);

                    crate::telemetry::log_network_stats(
                        smoothed_rtt_ms as u128, min_rtt_ms as u128, latest_rtt_ms as u128,
                        cwnd_bytes, bw_estimate, depth, app_limited, discard_ms,
                    );

                    crate::telemetry::log_abr_state(
                        "ACTIVE",
                        current_index,
                        depth as f64,
                        state.clear_streak as f64,
                        state.post_downshift_depth.is_some(),
                        state.post_downshift_depth.unwrap_or(0) as f64,
                    );
                }
            }
        }
    });
}