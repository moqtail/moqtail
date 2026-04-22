use std::sync::Arc;
use tracing::info;
use moqtail::model::data::full_track_name::FullTrackName;

use crate::server::client::MOQTClient;
use crate::server::track_manager::TrackManager;

// 👉 1. THE DATA OBJECT: Now with all BBR internal metrics
#[derive(Debug, Clone, Copy)]
pub struct NetworkStats {
    pub current_rtt_ms: u128,
    pub cwnd_bytes: u64,
    pub bbr_est_bw_bps: u64,
    pub pacing_rate_bps: u64,
    pub actual_send_rate_bps: u64, 
}

pub struct AbrState {
    pub current_index: usize,
}

// 👉 2. YOUR SANDBOX: The Open-Loop Observer
pub fn decide_quality(
    _stats: &NetworkStats, 
    _bitrates_bps: &[u128; 4], 
    _state: &mut AbrState
) -> usize {
    // HARDCODE TO 720p (Index 2)
    2 
}
pub(crate) fn start_abr_controller(
    client: Arc<MOQTClient>,
    track_manager: Arc<TrackManager>,
) {
    tokio::spawn(async move {
        let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started");
        let mut group_arrival_counts: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();

        let mut state = AbrState { current_index: 2 };
        let bitrates_bps: [u128; 4] = [500_000, 1_000_000, 2_500_000, 5_000_000];

        // 🔥 Start as None so we skip the fake math on the very first frame
        let mut last_check_time: Option<std::time::Instant> = None;
        let mut last_sent_bytes: u64 = 0;
        let mut phantom_buffer_bytes: f64 = 0.0;

        info!("Open-Loop Observer (720p) with Leaky Bucket Latency tracking started.");

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

            let current_sent_bytes = client.connection.stats().udp_tx.bytes;
            let now = std::time::Instant::now();

            // 🔥 Gracefully handle the time delta
            let duration_sec = match last_check_time {
                Some(last_time) => now.duration_since(last_time).as_secs_f64(),
                None => 0.0, // First frame has 0 duration
            };

            let bytes_sent = current_sent_bytes.saturating_sub(last_sent_bytes) as f64;
            let actual_send_rate_bps = if duration_sec > 0.0 {
                (bytes_sent * 8.0 / duration_sec) as u64
            } else {
                0
            };

            // Leaky Bucket Math (Will naturally calculate to 0 on the first frame)
            let selected_bps = bitrates_bps[state.current_index];
            let bytes_generated = (selected_bps as f64 / 8.0) * duration_sec;
            
            phantom_buffer_bytes += bytes_generated - bytes_sent;
            if phantom_buffer_bytes < 0.0 {
                phantom_buffer_bytes = 0.0; 
            }

            let phantom_latency_ms = ((phantom_buffer_bytes * 8.0) / selected_bps as f64) * 1000.0;
            let current_rtt_ms = client.connection.stats_rtt().as_millis();
            let total_latency_ms = current_rtt_ms + phantom_latency_ms as u128;

            let stats = NetworkStats {
                current_rtt_ms,
                cwnd_bytes: client.connection.cwnd(),
                bbr_est_bw_bps: client.connection.bandwidth().unwrap_or(0),
                pacing_rate_bps: client.connection.pacing_rate().unwrap_or(0),
                actual_send_rate_bps,
            };

            // 🔥 target_track is now safely in the main scope!
            let chosen_index = decide_quality(&stats, &bitrates_bps, &mut state);
            let target_track = &available_tracks[chosen_index];

            // Only log to telemetry if this isn't the fake initialization frame
            if last_check_time.is_some() {
                crate::telemetry::log_abr_decision(
                    group_id,
                    stats.bbr_est_bw_bps,
                    stats.pacing_rate_bps,
                    stats.actual_send_rate_bps,
                    stats.current_rtt_ms,
                    total_latency_ms,
                    stats.cwnd_bytes,
                    &target_track.to_string(),
                );
            }

            // Always update the trackers at the end of the loop
            last_check_time = Some(now);
            last_sent_bytes = current_sent_bytes;

            // -----------------------------------------------------
            // TRACK ALIASING 
            // -----------------------------------------------------
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

