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

//! Load generation: many subscriber sessions from one process, measuring what
//! arrives and how long it took from the publisher, and reporting it as JSON.
//!
//! The publisher stamps each object with its send time in the LOC Timestamp
//! property (`--timestamp`). Latency is the receive time minus that stamp, so the
//! publisher and the subscribers must share a clock: run them on one machine.

use crate::cli::Transport;
use crate::connection::MoqConnection;
use crate::subscriber::{self, SubscribeConfig};
use anyhow::Result;
use moqtail::model::common::pair::KeyValuePair;
use moqtail::model::property::constant::LOCPropertyId;
use moqtail::model::property::object_property::ObjectProperty;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// Microseconds since the Unix epoch on this machine's clock.
pub fn now_micros() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_micros() as u64)
    .unwrap_or(0)
}

/// The LOC Timestamp property carrying the send time.
pub fn send_timestamp() -> Option<ObjectProperty> {
  KeyValuePair::try_new_varint(LOCPropertyId::Timestamp as u64, now_micros())
    .and_then(ObjectProperty::deserialize)
    .ok()
}

/// What every subscriber session of a run has received. Shared by all sessions.
#[derive(Default)]
pub struct PerfRecorder {
  objects: AtomicU64,
  bytes: AtomicU64,
  gaps: AtomicU64,
  without_timestamp: AtomicU64,
  /// End-to-end latency of each timestamped object, in microseconds.
  latencies_us: Mutex<Vec<u64>>,
}

impl PerfRecorder {
  /// Counts one received object, and its latency when it carries a send time.
  pub fn record(&self, payload_len: usize, properties: Option<&Vec<ObjectProperty>>, gap: bool) {
    let received = now_micros();
    self.objects.fetch_add(1, Ordering::Relaxed);
    self.bytes.fetch_add(payload_len as u64, Ordering::Relaxed);
    if gap {
      self.gaps.fetch_add(1, Ordering::Relaxed);
    }
    match sent_at(properties) {
      Some(sent) => self
        .latencies_us
        .lock()
        .unwrap()
        .push(received.saturating_sub(sent)),
      None => {
        self.without_timestamp.fetch_add(1, Ordering::Relaxed);
      }
    }
  }
}

fn sent_at(properties: Option<&Vec<ObjectProperty>>) -> Option<u64> {
  properties?.iter().find_map(|p| match p {
    ObjectProperty::Unknown {
      kvp: KeyValuePair::VarInt { type_value, value },
    } if *type_value == LOCPropertyId::Timestamp as u64 => Some(*value),
    _ => None,
  })
}

/// How a load run is shaped.
pub struct LoadConfig {
  pub server: String,
  pub transport: Transport,
  pub no_cert_validation: bool,
  pub sessions: u32,
  /// Sessions started per second; 0 starts them all at once.
  pub ramp: u32,
  pub report: Option<String>,
}

/// Opens the sessions, each with its own connection and subscription, waits for
/// the subscribe duration to end, and writes the report.
pub async fn run(load: LoadConfig, subscribe: SubscribeConfig) -> Result<()> {
  let recorder = Arc::new(PerfRecorder::default());
  let connected = Arc::new(AtomicU64::new(0));
  let failed = Arc::new(AtomicU64::new(0));
  let ended_with_error = Arc::new(AtomicU64::new(0));
  let started = Instant::now();
  let duration = subscribe.duration;

  let mut tasks = Vec::new();
  for index in 0..load.sessions {
    if load.ramp > 0 && index > 0 && index % load.ramp == 0 {
      tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let mut config = subscribe.clone();
    config.perf = Some(recorder.clone());
    // Sessions started later stop at the same moment as the first ones.
    config.duration = duration.saturating_sub(started.elapsed().as_secs()).max(1);
    let (server, transport, insecure) =
      (load.server.clone(), load.transport, load.no_cert_validation);
    let (connected, failed, ended_with_error) =
      (connected.clone(), failed.clone(), ended_with_error.clone());
    tasks.push(tokio::spawn(async move {
      match MoqConnection::establish(&server, insecure, transport).await {
        Ok(moq) => {
          connected.fetch_add(1, Ordering::Relaxed);
          if let Err(e) = subscriber::run(moq, config).await {
            ended_with_error.fetch_add(1, Ordering::Relaxed);
            warn!("Session {index} ended with an error: {e:#}");
          }
        }
        Err(e) => {
          failed.fetch_add(1, Ordering::Relaxed);
          warn!("Session {index} could not connect: {e:#}");
        }
      }
    }));
  }
  for task in tasks {
    let _ = task.await;
  }

  let elapsed = started.elapsed().as_secs_f64();
  let report = build_report(
    &recorder,
    Sessions {
      attempted: load.sessions,
      connected: connected.load(Ordering::Relaxed),
      failed: failed.load(Ordering::Relaxed),
      ended_with_error: ended_with_error.load(Ordering::Relaxed),
    },
    elapsed,
  );
  info!("Load report: {report}");
  if let Some(path) = &load.report {
    std::fs::write(path, serde_json::to_string_pretty(&report)?)?;
  }
  Ok(())
}

/// How the sessions of a run fared. A session that connected but whose subscription
/// was refused, or that failed later, counts as connected and ended with an error.
struct Sessions {
  attempted: u32,
  connected: u64,
  failed: u64,
  ended_with_error: u64,
}

fn build_report(recorder: &PerfRecorder, sessions: Sessions, elapsed_s: f64) -> serde_json::Value {
  let mut latencies = recorder.latencies_us.lock().unwrap().clone();
  latencies.sort_unstable();
  let objects = recorder.objects.load(Ordering::Relaxed);
  let bytes = recorder.bytes.load(Ordering::Relaxed);
  serde_json::json!({
    "sessions": {
      "attempted": sessions.attempted,
      "connected": sessions.connected,
      "failed": sessions.failed,
      "ended_with_error": sessions.ended_with_error,
    },
    "elapsed_s": elapsed_s,
    "objects": objects,
    "bytes": bytes,
    "objects_per_s": objects as f64 / elapsed_s,
    "bytes_per_s": bytes as f64 / elapsed_s,
    "gaps": recorder.gaps.load(Ordering::Relaxed),
    "objects_without_timestamp": recorder.without_timestamp.load(Ordering::Relaxed),
    "latency_us": {
      "count": latencies.len(),
      "p50": percentile(&latencies, 0.50),
      "p95": percentile(&latencies, 0.95),
      "p99": percentile(&latencies, 0.99),
      "max": latencies.last().copied(),
    },
  })
}

/// The nearest-rank percentile of sorted values; `None` when there are none.
fn percentile(sorted: &[u64], q: f64) -> Option<u64> {
  if sorted.is_empty() {
    return None;
  }
  let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
  Some(sorted[rank - 1])
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn percentile_uses_nearest_rank() {
    let values: Vec<u64> = (1..=100).collect();
    assert_eq!(percentile(&values, 0.50), Some(50));
    assert_eq!(percentile(&values, 0.99), Some(99));
    assert_eq!(percentile(&values, 1.0), Some(100));
    assert_eq!(percentile(&[], 0.5), None);
  }

  #[test]
  fn records_latency_from_the_send_timestamp() {
    let recorder = PerfRecorder::default();
    let stamped = vec![send_timestamp().unwrap()];
    recorder.record(100, Some(&stamped), false);
    recorder.record(50, None, true);
    let sessions = Sessions {
      attempted: 1,
      connected: 1,
      failed: 0,
      ended_with_error: 0,
    };
    let report = build_report(&recorder, sessions, 1.0);
    assert_eq!(report["objects"], 2);
    assert_eq!(report["bytes"], 150);
    assert_eq!(report["gaps"], 1);
    assert_eq!(report["objects_without_timestamp"], 1);
    assert_eq!(report["latency_us"]["count"], 1);
  }
}
