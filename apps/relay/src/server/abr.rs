use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::info;
use moqtail::model::data::full_track_name::FullTrackName;

use crate::server::client::MOQTClient;
use crate::server::track_manager::TrackManager;

pub struct AbrState {
    pub current_index: usize,
    pub stable_groups: u64,
    pub blacklisted_index: Option<usize>,
    pub groups_since_blacklist: u64,
    pub total_groups_processed: u64,
    pub consecutive_high_backlog: u64,
}

/// Maps the current quality index to a stream discard timeout.
///
/// Lower quality means we are already shedding load — discard stale streams
/// faster to keep the queue clear. Higher quality means the network is healthy
/// and we can afford to wait longer for buffers to flush cleanly.
fn discard_timeout_for_index(index: usize) -> u64 {
    match index {
        0 => 150, // 360p — emergency / heavily congested
        1 => 250, // 480p — degraded
        2 => 400, // 720p — moderate
        _ => 500, // 1080p — healthy
    }
}

pub fn decide_quality(
    group_backlog: u64,
    state: &mut AbrState,
) -> usize {
    state.total_groups_processed += 1;

    // ==========================================
    // 1. COLD START GRACE PERIOD
    // ==========================================
    // BBR is aggressive in the first 5 seconds. Hold 1080p and ignore backlog noise.
    if state.total_groups_processed <= 10 {
        state.current_index = 3;
        return state.current_index;
    }

    // ==========================================
    // 2. BLACKLIST FORGIVENESS
    // ==========================================
    if let Some(bad_index) = state.blacklisted_index {
        state.groups_since_blacklist += 1;

        // Timeout: 30 groups = ~15 seconds.
        if state.groups_since_blacklist >= 30 {
            tracing::info!("TIMEOUT: Forgiving blacklisted index {}.", bad_index);
            state.blacklisted_index = None;
            state.groups_since_blacklist = 0;
        }
    }

    // ==========================================
    // 3. DEBOUNCED ALARM (Sustained Backlog)
    // ==========================================
    if group_backlog >= 2 {
        state.consecutive_high_backlog += 1;
    } else {
        // Queue drained — momentary I-frame burst, not real congestion.
        state.consecutive_high_backlog = 0;
    }

    // Only react if the backlog stays elevated for 3 consecutive decisions.
    if state.consecutive_high_backlog >= 3 {
        state.blacklisted_index = Some(state.current_index);
        state.groups_since_blacklist = 0;
        state.current_index = state.current_index.saturating_sub(1);
        state.stable_groups = 0;
        state.consecutive_high_backlog = 0;

        tracing::warn!(
            "SUSTAINED CONGESTION: Queue jammed. Blacklisting index {}. Dropping to index {}.",
            state.blacklisted_index.unwrap(),
            state.current_index
        );
        return state.current_index;
    }

    // ==========================================
    // 4. CAUTIOUS CLIMB
    // ==========================================
    if group_backlog == 0 {
        state.stable_groups += 1;
    } else {
        // Queue at 1 — don't panic, but pause the upgrade timer.
        state.stable_groups = 0;
    }

    // Step up every 10 groups (~5 seconds) of perfect stability.
    if state.stable_groups >= 10 {
        let next_index = state.current_index + 1;

        if next_index <= 3 {
            if Some(next_index) == state.blacklisted_index {
                // Hold the ceiling — don't jump into the blacklisted tier.
                state.stable_groups = 0;
            } else {
                state.current_index = next_index;
                state.stable_groups = 0;
                tracing::info!("PROBE: Backlog clear. Upgrading to index {}.", state.current_index);
            }
        }
    }

    state.current_index
}

pub(crate) fn start_abr_controller(
    client: Arc<MOQTClient>,
    track_manager: Arc<TrackManager>,
) {
    tokio::spawn(async move {
        let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started");

        let mut state = AbrState {
            current_index: 3,
            stable_groups: 0,
            blacklisted_index: None,
            groups_since_blacklist: 0,
            total_groups_processed: 0,
            consecutive_high_backlog: 0,
        };

        let mut log_interval = tokio::time::interval(std::time::Duration::from_millis(100));
        let mut last_decided_group = 0u64;

        info!("ABR controller started.");

        loop {
            tokio::select! {
                // ==========================================
                // BRANCH 1: ONE DECISION PER GROUP
                // ==========================================
                msg = abr_rx.recv() => {
                    let group_id = match msg {
                        Some(id) => id,
                        None => break,
                    };

                    // Catastrophic flare sent from close_stream when a stream reset occurs.
                    if group_id == u64::MAX {
                        tracing::warn!("PANIC FLARE: Stream reset detected. Blacklisting current index and slamming to 360p.");
                        state.blacklisted_index = Some(state.current_index);
                        state.groups_since_blacklist = 0;
                        state.current_index = 0;
                        state.stable_groups = 0;
                        state.consecutive_high_backlog = 0;
                        client.discard_timeout_ms.store(
                            discard_timeout_for_index(state.current_index),
                            Ordering::Relaxed,
                        );
                        continue;
                    }

                    if group_id <= last_decided_group {
                        continue;
                    }
                    last_decided_group = group_id;

                    let mut available_tracks: Vec<FullTrackName> =
                        track_manager.track_aliases.read().await.values().cloned().collect();
                    if available_tracks.is_empty() {
                        continue;
                    }

                    available_tracks.sort_by_key(|t| {
                        let name = format!("{}", t);
                        if name.contains("360p") { 0 }
                        else if name.contains("480p") { 1 }
                        else if name.contains("720p") { 2 }
                        else if name.contains("1080p") { 3 }
                        else { 4 }
                    });

                    let group_backlog = client.active_send_tasks.load(Ordering::SeqCst) as u64;
                    let prev_index = state.current_index;
                    let chosen_index = decide_quality(group_backlog, &mut state);

                    // Propagate discard timeout whenever the quality index changes.
                    if state.current_index != prev_index {
                        client.discard_timeout_ms.store(
                            discard_timeout_for_index(state.current_index),
                            Ordering::Relaxed,
                        );
                    }

                    let safe_index = chosen_index.min(available_tracks.len().saturating_sub(1));
                    let target_track = &available_tracks[safe_index];

                    crate::telemetry::log_abr_decision(group_id, &target_track.to_string());

                    let target_alias = {
                        let aliases = track_manager.track_aliases.read().await;
                        aliases.iter().find(|(_, name)| *name == target_track).map(|(alias, _)| *alias)
                    };

                    if let Some(alias) = target_alias {
                        let mut decisions = client.group_decisions.write().await;
                        for g in group_id.saturating_sub(3)..=group_id {
                            decisions.insert(g, alias);
                            let _ = client.decision_tx.send(g);
                        }
                        decisions.retain(|&k, _| k >= group_id.saturating_sub(5));
                    }
                }

                // ==========================================
                // BRANCH 2: CONTINUOUS NETWORK METRICS
                // ==========================================
                _ = log_interval.tick() => {
                    if last_decided_group == 0 {
                        continue;
                    }

                    let group_backlog = client.active_send_tasks.load(Ordering::SeqCst) as u64;

                    // Emergency: backlog has grown far beyond what the network can clear.
                    if group_backlog >= 8 {
                        state.blacklisted_index = Some(state.current_index);
                        state.groups_since_blacklist = 0;
                        state.current_index = 0;
                        state.stable_groups = 0;
                        state.consecutive_high_backlog = 0;

                        client.discard_timeout_ms.store(
                            discard_timeout_for_index(state.current_index),
                            Ordering::Relaxed,
                        );

                        let mut decisions = client.group_decisions.write().await;
                        let stuck_group = last_decided_group.saturating_sub(1);
                        decisions.insert(stuck_group, u64::MAX);
                        let _ = client.decision_tx.send(stuck_group);
                    }

                    // Read the four CC signals we exposed from quinn.
                    // app_limited == true means the sender had no data queued at the last tx
                    // opportunity, so RTT/cwnd readings may not reflect true network capacity.
                    let smoothed_rtt_ms = client.connection.rtt().as_millis();
                    let min_rtt_ms = client.connection.min_rtt().as_millis();
                    let latest_rtt_ms = client.connection.latest_rtt().as_millis();
                    let cwnd = client.connection.cwnd();
                    let app_limited = client.connection.app_limited();
                    let discard_timeout_ms = client.discard_timeout_ms.load(Ordering::Relaxed);

                    crate::telemetry::log_network_stats(
                        smoothed_rtt_ms,
                        min_rtt_ms,
                        latest_rtt_ms,
                        cwnd,
                        group_backlog,
                        app_limited,
                        discard_timeout_ms,
                    );
                }
            }
        }
    });
}
