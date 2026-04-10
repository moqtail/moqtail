
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json::json;
use tracing::error;

/// Appends a telemetry event to a local file in JSONL format.
pub fn log_abr_decision(group_id: u64, est_bw: u64, rtt_ms: u128, selected_track: &str) {
    // Get the shared kernel time in microseconds
    let now_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();

    let log_entry = json!({
        "timestamp_us": now_us as u64,
        "event_type": "ABR_DECISION",
        "group_id": group_id,
        "bbr_est_bw_bps": est_bw,
        "bbr_rtt_ms": rtt_ms,
        "selected_track": selected_track
    });

    // Attempt to open and append to the file
    match OpenOptions::new().create(true).append(true).open("relay_telemetry.jsonl") {
        Ok(mut file) => {
            if let Err(e) = writeln!(file, "{}", log_entry) {
                error!("Failed to write to telemetry log: {}", e);
            }
        }
        Err(e) => {
            error!("Failed to open telemetry log file: {}", e);
        }
    }
}