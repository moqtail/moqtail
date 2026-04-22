use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json::json;
use tracing::error;

/// Appends a telemetry event to a local file in JSONL format.
pub fn log_abr_decision(
    group_id: u64, 
    bbr_est_bw_bps: u64, 
    pacing_rate_bps: u64, 
    actual_send_rate_bps: u64, 
    rtt_ms: u128, 
    total_latency_ms: u128, // 🔥 NEW
    cwnd_bytes: u64, 
    selected_track: &str
) {
    let now_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();

    let log_entry = json!({
        "timestamp_us": now_us as u64,
        "event_type": "ABR_DECISION",
        "group_id": group_id,
        "bbr_est_bw_bps": bbr_est_bw_bps,
        "pacing_rate_bps": pacing_rate_bps,
        "actual_send_rate_bps": actual_send_rate_bps,
        "bbr_rtt_ms": rtt_ms,
        "total_latency_ms": total_latency_ms, // 🔥 NEW
        "cwnd_bytes": cwnd_bytes,
        "selected_track": selected_track
    });

    match OpenOptions::new().create(true).append(true).open("logs/relay_telemetry.jsonl") {
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