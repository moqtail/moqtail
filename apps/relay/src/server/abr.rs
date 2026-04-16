use std::sync::Arc;
use tracing::{info, warn};
use moqtail::model::data::full_track_name::FullTrackName;

use crate::server::client::MOQTClient;
use crate::server::track_manager::TrackManager;

pub(crate) fn start_abr_controller(
    client: Arc<MOQTClient>,
    track_manager: Arc<TrackManager>,
) {
    tokio::spawn(async move {
        let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started");

        let mut min_rtt: u128 = u128::MAX;
        let mut group_arrival_counts: std::collections::HashMap<u64, usize> =
            std::collections::HashMap::new();

        let mut current_track_index: usize = 3;
        let mut hold_timer_groups: u8 = 0;
        let step_up_delay: u8 = 4; // Wait 4 groups (2 seconds) before probing higher

        info!(
            "Queue-Based AIMD ABR Controller started for Client ID: {}",
            client.connection_id
        );

        let step_up_delay: u8 = 4; // Wait 4 groups (2 seconds) before probing higher

        // 👉 THE MISSING ARRAY
        let bitrates_bps: [u128; 4] = [
            500_000,   // Index 0: 360p (0.5 Mbps)
            1_000_000, // Index 1: 480p (1.0 Mbps)
            2_500_000, // Index 2: 720p (2.5 Mbps)
            5_000_000  // Index 3: 1080p (5.0 Mbps)
        ];

        info!(
            "Queue-Based AIMD ABR Controller started for Client ID: {}",
            client.connection_id
        );

        while let Some(group_id) = abr_rx.recv().await {
            let mut available_tracks: Vec<FullTrackName> =
                track_manager
                    .track_aliases
                    .read()
                    .await
                    .values()
                    .cloned()
                    .collect();

            if available_tracks.len() < 4 {
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

            // Barrier Gate
            let count = group_arrival_counts.entry(group_id).or_insert(0);
            *count += 1;
            if *count < 4 {
                continue;
            }
            group_arrival_counts.retain(|&k, _| k >= group_id);

            // 1. Calculate Queue Debt
            let current_rtt = client.connection.stats_rtt().as_millis();
            if current_rtt > 0 && current_rtt < min_rtt {
                min_rtt = current_rtt;
            }
            let queueing_delay = if current_rtt > min_rtt { current_rtt - min_rtt } else { 0 };

            // 2. Fetch BBR Estimate
            let est_bw_bps = client.connection.bandwidth().unwrap_or(0) as u128;
            let current_track_bitrate = bitrates_bps[current_track_index];

            // 👉 THE HARD CEILING CHECK
            // If BBR is estimating less bandwidth than we are currently sending, 
            // the pipe is definitively choked, even if RTT is lying to us due to packet drops.
            let is_bw_choked = est_bw_bps > 0 && est_bw_bps < current_track_bitrate;

           // 👉 ULL-OPTIMIZED AIMD ENGINE (1.5s Budget)
            // If normal latency is ~200ms, we must give the algorithm room to breathe.
            let safe_zone_ms: u128 = 400;   // Green: Safe to probe upward
            let danger_zone_ms: u128 = 800; // Yellow: Hold line / Red: Step down
            let panic_zone_ms: u128 = 1200; // Black: Imminent 1.5s budget violation

            if queueing_delay > panic_zone_ms {
                // 🚨 PANIC DROP: We are about to blow the 1.5s limit. Floor it.
                if hold_timer_groups == 0 {
                    warn!("⚠️ ABR PANIC: Queue Debt {} ms. Dropping to 360p.", queueing_delay);
                }
                current_track_index = 0; 
                // Massive penalty: wait 6 groups (3 seconds) before trying to step up again
                hold_timer_groups = step_up_delay + 2; 

            } else if is_bw_choked || queueing_delay > danger_zone_ms {
                // 📉 DANGER ZONE: Ratchet down. 
                // Either the queue crossed 800ms, OR BBR detected a massive capacity drop.
                if current_track_index > 0 {
                    current_track_index -= 1;
                    warn!("📉 DANGER: Choked. BBR: {} bps, Queue: {} ms. Stepping to index {}.", 
                          est_bw_bps, queueing_delay, current_track_index);
                }
                // Only give it 1 group of breathing room before it evaluates dropping again
                hold_timer_groups = 1; 

            } else if queueing_delay > safe_zone_ms {
                // ⚠️ YELLOW ZONE (400ms - 800ms): The pipe is working hard. 
                // Do not step up, do not step down. Let BBR stabilize and let the queue drain.
                hold_timer_groups = step_up_delay; 

            } else {
                // 📈 GREEN ZONE (0ms - 400ms): Pipe is clean. Go for maximum quality.
                if hold_timer_groups > 0 {
                    hold_timer_groups -= 1;
                } else {
                    if current_track_index < 3 {
                        current_track_index += 1;
                        hold_timer_groups = step_up_delay; // Reset timer for the next probe
                    }
                }
            }

            let target_track = &available_tracks[current_track_index];
            crate::telemetry::log_abr_decision(
                group_id,
                est_bw_bps as u64,
                current_rtt,
                &target_track.to_string(),
            );

            // Unlock the gate
            let target_alias = {
                let aliases = track_manager.track_aliases.read().await;
                aliases
                    .iter()
                    .find(|(_, name)| *name == target_track)
                    .map(|(alias, _)| *alias)
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