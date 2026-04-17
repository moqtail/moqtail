use std::sync::Arc;
use tracing::{info, warn};
use moqtail::model::data::full_track_name::FullTrackName;

use crate::server::client::MOQTClient;
use crate::server::track_manager::TrackManager;

// 👉 1. THE DATA OBJECT
#[derive(Debug, Clone, Copy)]
pub struct NetworkStats {
    pub current_rtt_ms: u128,
    pub bbr_est_bw_bps: u64,
    pub actual_send_rate_bps: u64, // The MTU-hacked absolute truth
}

// 👉 2. THE MEMORY
pub struct AbrState {
    pub current_index: usize,
    pub last_rtt_ms: u128,        // Brought back for growth tracking
    pub just_stepped_up: bool,    // Brought back for probe failure tracking
    pub wait_timer: u32,
    pub current_backoff: u32,
}

// 👉 3. YOUR SANDBOX: Send-Rate Trigger + Send-Rate Landing
pub fn decide_quality(
    stats: &NetworkStats, 
    bitrates_bps: &[u128; 4], 
    state: &mut AbrState
) -> usize {
    
    let target_bitrate = bitrates_bps[state.current_index] as u64;
    let rtt_growth = stats.current_rtt_ms.saturating_sub(state.last_rtt_ms);

    // LOGIC LINE 1: The BW Drop / Buffer Bloat Trigger
    // If the physical wire is draining 20% slower than our current video track,
    // OR if the RTT has breached our absolute safety limit (250ms), we are congested.
    if (stats.actual_send_rate_bps > 0 && stats.actual_send_rate_bps < (target_bitrate * 8 / 10)) 
        || stats.current_rtt_ms > 250 
    {
        let mut safe_index = 0; 
        
        // Find the Landing Zone using the REAL send rate
        for i in (0..=state.current_index).rev() {
            if bitrates_bps[i] < stats.actual_send_rate_bps as u128 {
                safe_index = i;
                break;
            }
        }
        
        state.current_index = safe_index;
        state.wait_timer = state.current_backoff + 2; // Heavy penalty for causing congestion
        state.just_stepped_up = false;
    }
    // LOGIC LINE 2: The Failed Probe (Micro-Congestion)
    // If we just pushed the throttle and RTT instantly bumped, pull back safely.
    else if state.just_stepped_up && rtt_growth > 15 {
        if state.current_index > 0 {
            state.current_index -= 1;
        }
        state.current_backoff *= 2;               
        state.wait_timer = state.current_backoff; 
        state.just_stepped_up = false;
    }
    // LOGIC LINE 3: Safe to Probe
    else {
        if state.wait_timer > 0 {
            state.wait_timer -= 1;
            state.just_stepped_up = false; 
        } else {
            // Timer expired! Probe up.
            if state.current_index < 3 {
                state.current_index += 1;
                state.just_stepped_up = true;             
                state.wait_timer = state.current_backoff; 
            }
        }
    }
    
    // Save state for next tick
    state.last_rtt_ms = stats.current_rtt_ms;
    
    state.current_index
}

// 👉 4. THE BOILERPLATE
pub(crate) fn start_abr_controller(
    client: Arc<MOQTClient>,
    track_manager: Arc<TrackManager>,
) {
    tokio::spawn(async move {
        let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started");
        let mut group_arrival_counts: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();

        let mut state = AbrState {
            current_index: 3,     
            last_rtt_ms: 0,
            just_stepped_up: false,
            wait_timer: 0,
            current_backoff: 4,   
        };
        let bitrates_bps: [u128; 4] = [
            500_000, 1_000_000, 2_500_000, 5_000_000 
        ];

        // --- NEW: Trackers for the True Send Rate ---
        let mut last_check_time = std::time::Instant::now();
        let mut last_sent_bytes = 0;

        while let Some(group_id) = abr_rx.recv().await {
            let mut available_tracks: Vec<FullTrackName> = track_manager.track_aliases.read().await.values().cloned().collect();
            if available_tracks.len() < 4 { continue; }
            available_tracks.sort_by_key(|t| {
                let name = format!("{}", t);
                if name.contains("360p") { 0 } else if name.contains("480p") { 1 } 
                else if name.contains("720p") { 2 } else if name.contains("1080p") { 3 } else { 4 }
            });

            let count = group_arrival_counts.entry(group_id).or_insert(0);
            *count += 1;
            if *count < 4 { continue; }
            group_arrival_counts.retain(|&k, _| k >= group_id); 

            // --- CALCULATE TRUE SEND RATE ---
            let conn_stats = client.connection.stats();
            let current_sent_packets = conn_stats.path.sent_packets;
            let now = std::time::Instant::now();
            let duration_sec = now.duration_since(last_check_time).as_secs_f64();

            let current_sent_bytes = 1500 * current_sent_packets; //fuck it, most packets are mtu
            let actual_send_rate_bps = if duration_sec > 0.0 {
                // (Bytes sent since last check * 8 bits) / time elapsed
                ((current_sent_bytes.saturating_sub(last_sent_bytes)) as f64 * 8.0 / duration_sec) as u64
            } else {
                0
            };

            last_check_time = now;
            last_sent_bytes = current_sent_bytes;
            // --------------------------------

            let stats = NetworkStats {
                current_rtt_ms: client.connection.stats_rtt().as_millis(),
                bbr_est_bw_bps: client.connection.bandwidth().unwrap_or(0),
                actual_send_rate_bps,
            };

            let chosen_index = decide_quality(&stats, &bitrates_bps, &mut state);
            let target_track = &available_tracks[chosen_index];

            // (Optional) You can log actual_send_rate_bps here to your visualizer instead of BBR!
            crate::telemetry::log_abr_decision(
                group_id,
                stats.actual_send_rate_bps, // Passing our new truth metric instead of BBR!
                stats.current_rtt_ms,
                conn_stats.path.cwnd, 
                &target_track.to_string(),
            );

            let target_alias = {
                let aliases = track_manager.track_aliases.read().await;
                aliases.iter().find(|(_, name)| *name == target_track).map(|(alias, _)| *alias)
            };

            if let Some(alias) = target_alias {
                let mut decisions = client.group_decisions.write().await;
                decisions.insert(group_id, alias);
                decisions.retain(|&k, _| k >= group_id.saturating_sub(5)); 
            }

            let _ = client.decision_tx.send(group_id);
        }
    });
}