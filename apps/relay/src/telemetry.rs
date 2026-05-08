use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json::json;
use tracing::error;

/// Appends a telemetry event to a local file in JSONL format.
pub fn log_abr_decision(
    group_id: u64,
    selected_track: &str,
) {
    let now_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();

    let log_entry = json!({
        "timestamp_us": now_us as u64,
        "event_type": "ABR_DECISION",
        "group_id": group_id,
        "selected_track": selected_track,
    });

    write_entry(&log_entry);
}

pub fn log_network_stats(
    smoothed_rtt_ms: u128,
    min_rtt_ms: u128,
    latest_rtt_ms: u128,
    cwnd_bytes: u64,
    group_backlog: u64,
    app_limited: bool,
    discard_timeout_ms: u64,
) {
    let now_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();

    let log_entry = json!({
        "timestamp_us": now_us as u64,
        "event_type": "NETWORK_STAT",
        "smoothed_rtt_ms": smoothed_rtt_ms,
        "min_rtt_ms": min_rtt_ms,
        "latest_rtt_ms": latest_rtt_ms,
        "cwnd_bytes": cwnd_bytes,
        "group_backlog": group_backlog,
        "app_limited": app_limited,
        "discard_timeout_ms": discard_timeout_ms,
    });

    write_entry(&log_entry);
}

fn write_entry(entry: &serde_json::Value) {
    match OpenOptions::new().create(true).append(true).open("logs/relay_telemetry.jsonl") {
        Ok(mut file) => {
            if let Err(e) = writeln!(file, "{}", entry) {
                error!("Failed to write to telemetry log: {}", e);
            }
        }
        Err(e) => {
            error!("Failed to open telemetry log file: {}", e);
        }
    }
}
