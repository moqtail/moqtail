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

//! `/api/v1/metrics`: everything the API counts, in the Prometheus text format.
//!
//! Per-connection and per-track numbers are left out on purpose: their number has no
//! bound, which Prometheus handles badly. They are in `/connections` and `/tracks`.

use crate::ApiState;
use crate::list::parse_query;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use moqtail_probe::stats::{BUCKETS, bucket_bounds_micros};
use moqtail_probe::{ConnectionState, Role};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Arc;

const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

#[derive(Debug, Default, Deserialize)]
pub struct MetricsQuery {
  /// Only metrics whose name contains this.
  filter: Option<String>,
}

pub async fn metrics(
  State(state): State<Arc<ApiState>>,
  query: Result<Query<MetricsQuery>, QueryRejection>,
) -> Result<impl IntoResponse, crate::error::ApiError> {
  let query = parse_query(query)?;
  let mut out = Exposition::new(query.filter);
  render(&state, &mut out).await;
  Ok(([(header::CONTENT_TYPE, CONTENT_TYPE)], out.text))
}

/// Builds the text, one metric family at a time, skipping families the filter
/// leaves out.
struct Exposition {
  filter: Option<String>,
  text: String,
}

impl Exposition {
  fn new(filter: Option<String>) -> Self {
    Exposition {
      filter,
      text: String::new(),
    }
  }

  fn wanted(&self, name: &str) -> bool {
    self.filter.as_deref().is_none_or(|f| name.contains(f))
  }

  /// One family: its help, its type, and its samples as (labels, value).
  fn family(&mut self, name: &str, kind: &str, help: &str, samples: &[(String, f64)]) {
    if !self.wanted(name) || samples.is_empty() {
      return;
    }
    let _ = writeln!(self.text, "# HELP {name} {help}");
    let _ = writeln!(self.text, "# TYPE {name} {kind}");
    for (labels, value) in samples {
      if labels.is_empty() {
        let _ = writeln!(self.text, "{name} {value}");
      } else {
        let _ = writeln!(self.text, "{name}{{{labels}}} {value}");
      }
    }
  }

  fn one(&mut self, name: &str, kind: &str, help: &str, value: f64) {
    self.family(name, kind, help, &[(String::new(), value)]);
  }

  /// A histogram from bucket counts, in seconds. The bucket bounds carry the data;
  /// no sum is kept, so there is no `_sum` line.
  fn histogram(&mut self, name: &str, help: &str, series: &[(String, [u64; BUCKETS])]) {
    if !self.wanted(name) || series.is_empty() {
      return;
    }
    let bounds = bucket_bounds_micros();
    let _ = writeln!(self.text, "# HELP {name} {help}");
    let _ = writeln!(self.text, "# TYPE {name} histogram");
    for (labels, counts) in series {
      let sep = if labels.is_empty() { "" } else { "," };
      let mut cumulative = 0;
      for (i, count) in counts.iter().enumerate() {
        cumulative += count;
        let le = if i == BUCKETS - 1 {
          "+Inf".to_string()
        } else {
          format!("{}", bounds[i] as f64 / 1_000_000.0)
        };
        let _ = writeln!(
          self.text,
          "{name}_bucket{{{labels}{sep}le=\"{le}\"}} {cumulative}"
        );
      }
      if labels.is_empty() {
        let _ = writeln!(self.text, "{name}_count {cumulative}");
      } else {
        let _ = writeln!(self.text, "{name}_count{{{labels}}} {cumulative}");
      }
    }
  }
}

fn label(pairs: &[(&str, &str)]) -> String {
  pairs
    .iter()
    .map(|(k, v)| format!("{k}=\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"")))
    .collect::<Vec<_>>()
    .join(",")
}

async fn render(state: &ApiState, out: &mut Exposition) {
  let status = state.inspect.status();
  out.family(
    "moqtail_build_info",
    "gauge",
    "The relay's version; always 1.",
    &[(label(&[("version", &status.version)]), 1.0)],
  );
  out.one(
    "process_start_time_seconds",
    "gauge",
    "When the relay started, in seconds since the Unix epoch.",
    status.started_at_ms as f64 / 1000.0,
  );
  if let Some(rss) = resident_memory_bytes() {
    out.one(
      "process_resident_memory_bytes",
      "gauge",
      "Resident memory of the relay process.",
      rss as f64,
    );
  }
  if let Some(fds) = open_fds() {
    out.one(
      "process_open_fds",
      "gauge",
      "Open file descriptors of the relay process.",
      fds as f64,
    );
  }

  let connections = state.open_connections().await;
  let mut by_kind: BTreeMap<(String, &str, &str), u64> = BTreeMap::new();
  let mut rtts: Vec<f64> = Vec::new();
  let mut lost = 0;
  for c in connections.iter() {
    let role = match c.role {
      Role::Client => "client",
      Role::ClusterPeer => "cluster_peer",
    };
    let conn_state = match c.state {
      ConnectionState::Active => "active",
      ConnectionState::Idle => "idle",
      ConnectionState::Draining => "draining",
      ConnectionState::Closed => "closed",
    };
    *by_kind
      .entry((c.transport.clone(), role, conn_state))
      .or_default() += 1;
    rtts.push(c.quic.rtt_ms / 1000.0);
    lost += c.quic.lost_packets;
  }
  out.family(
    "moqtail_connections",
    "gauge",
    "Open connections.",
    &by_kind
      .iter()
      .map(|((t, r, s), n)| {
        (
          label(&[("transport", t), ("role", r), ("state", s)]),
          *n as f64,
        )
      })
      .collect::<Vec<_>>(),
  );
  rtts.sort_by(f64::total_cmp);
  if !rtts.is_empty() {
    let at = |q: f64| rtts[((q * rtts.len() as f64).ceil() as usize).clamp(1, rtts.len()) - 1];
    out.family(
      "moqtail_quic_rtt_seconds",
      "gauge",
      "Smoothed RTT across the open connections, at a quantile.",
      &[
        (label(&[("quantile", "0.5")]), at(0.5)),
        (label(&[("quantile", "0.99")]), at(0.99)),
      ],
    );
  }
  out.one(
    "moqtail_quic_lost_packets",
    "gauge",
    "Packets lost so far by the open connections.",
    lost as f64,
  );

  let closes: Vec<(String, f64)> = state
    .probe
    .closes
    .lock()
    .unwrap()
    .iter()
    .map(|((by, code), n)| (label(&[("by", by), ("code", code)]), *n as f64))
    .collect();
  out.family(
    "moqtail_connection_closes_total",
    "counter",
    "Closed connections, by who closed them and the code.",
    &closes,
  );
  let failures = counted(&state.probe.failures, "reason");
  out.family(
    "moqtail_connection_failures_total",
    "counter",
    "Connections that failed before becoming a session.",
    &failures,
  );
  let limits = counted(&state.probe.limit_rejections, "limit");
  out.family(
    "moqtail_limit_rejections_total",
    "counter",
    "Requests refused or shed because of a limit.",
    &limits,
  );

  let totals = state.control.totals();
  out.family(
    "moqtail_control_messages_total",
    "counter",
    "Control messages, by direction and type.",
    &totals
      .messages
      .iter()
      .map(|((d, t), n)| (label(&[("direction", d), ("type", t)]), *n as f64))
      .collect::<Vec<_>>(),
  );
  out.one(
    "moqtail_control_parse_errors_total",
    "counter",
    "Control message bytes that did not parse.",
    totals.parse_errors as f64,
  );
  let mut requests = Vec::new();
  for (kind, direction, results) in state.control.results_by_kind() {
    requests.push((
      label(&[("type", kind), ("direction", direction), ("result", "ok")]),
      results.ok as f64,
    ));
    for (code, n) in results.error {
      let code = code.to_string();
      requests.push((
        label(&[("type", kind), ("direction", direction), ("result", &code)]),
        n as f64,
      ));
    }
  }
  out.family(
    "moqtail_requests_total",
    "counter",
    "Answered requests by type and direction; result is ok or the error code.",
    &requests,
  );
  out.one(
    "moqtail_requests_in_flight",
    "gauge",
    "Requests waiting for an answer.",
    state.control.in_flight() as f64,
  );
  out.family(
    "moqtail_upstream_requests_total",
    "counter",
    "Requests the relay sent to publishers and peers, by result.",
    &[
      (label(&[("result", "ok")]), totals.upstream_ok as f64),
      (
        label(&[("result", "failed")]),
        totals.upstream_failed as f64,
      ),
      (
        label(&[("result", "timed_out")]),
        totals.upstream_timed_out as f64,
      ),
    ],
  );
  out.histogram(
    "moqtail_request_duration_seconds",
    "Time from a request to its answer, by type and direction.",
    &state
      .control
      .durations()
      .into_iter()
      .map(|(kind, direction, counts)| (label(&[("type", kind), ("direction", direction)]), counts))
      .collect::<Vec<_>>(),
  );

  let tracks = state.tracks().await;
  let mut origins: BTreeMap<&str, u64> = BTreeMap::new();
  for t in tracks.iter() {
    *origins.entry(t.origin.as_str()).or_default() += 1;
  }
  out.family(
    "moqtail_tracks",
    "gauge",
    "Tracks, by how they came to the relay.",
    &origins
      .iter()
      .map(|(o, n)| (label(&[("origin", o)]), *n as f64))
      .collect::<Vec<_>>(),
  );
  out.one(
    "moqtail_subscriptions",
    "gauge",
    "Subscriptions held by open connections.",
    connections
      .iter()
      .map(|c| c.subscriptions.len())
      .sum::<usize>() as f64,
  );
  out.one(
    "moqtail_cache_groups",
    "gauge",
    "Groups in the cache, over all tracks.",
    tracks.iter().map(|t| t.cached_groups).sum::<u64>() as f64,
  );

  if let Some(c) = &state.counters {
    out.one(
      "moqtail_objects_received_total",
      "counter",
      "Objects received from publishers.",
      c.objects_received.get() as f64,
    );
    out.one(
      "moqtail_received_bytes_total",
      "counter",
      "Payload bytes received from publishers.",
      c.bytes_received.get() as f64,
    );
    out.family(
      "moqtail_objects_sent_total",
      "counter",
      "Objects written to subscribers, live or as a FETCH response.",
      &[
        (label(&[("delivery", "live")]), c.objects_sent.get() as f64),
        (
          label(&[("delivery", "fetch")]),
          c.fetch_objects_sent.get() as f64,
        ),
      ],
    );
    out.family(
      "moqtail_sent_bytes_total",
      "counter",
      "Payload bytes written to subscribers, live or as a FETCH response.",
      &[
        (label(&[("delivery", "live")]), c.bytes_sent.get() as f64),
        (
          label(&[("delivery", "fetch")]),
          c.fetch_bytes_sent.get() as f64,
        ),
      ],
    );
    out.family(
      "moqtail_streams_total",
      "counter",
      "Data streams opened to subscribers and accepted from publishers.",
      &[
        (
          label(&[("direction", "to_subscriber")]),
          c.streams_opened.get() as f64,
        ),
        (
          label(&[("direction", "from_publisher")]),
          c.streams_accepted.get() as f64,
        ),
      ],
    );
    out.family(
      "moqtail_objects_dropped_total",
      "counter",
      "Objects not delivered to a subscriber, by reason.",
      &[
        (
          label(&[("reason", "queue_full")]),
          c.dropped_queue_full.get() as f64,
        ),
        (
          label(&[("reason", "delivery_timeout")]),
          c.dropped_delivery_timeout.get() as f64,
        ),
      ],
    );
    out.one(
      "moqtail_subscriptions_too_far_behind_total",
      "counter",
      "Subscriptions reset because they fell too far behind.",
      c.subscriptions_too_far_behind.get() as f64,
    );
    out.family(
      "moqtail_cache_removals_total",
      "counter",
      "Groups that left the cache, by cause.",
      &[
        (label(&[("cause", "expired")]), c.cache_expired.get() as f64),
        (
          label(&[("cause", "size")]),
          c.cache_evicted_for_size.get() as f64,
        ),
        (label(&[("cause", "removed")]), c.cache_removed.get() as f64),
      ],
    );
  }

  if let Some(latency) = &state.probe.latency {
    let r = &latency.relay;
    out.histogram(
      "moqtail_object_processing_seconds",
      "Time an object spends in the relay on its way to one subscriber, by stage.",
      &[
        (label(&[("stage", "ingest")]), r.ingest.counts()),
        (label(&[("stage", "queue")]), r.queue.counts()),
        (label(&[("stage", "write")]), r.write.counts()),
        (label(&[("stage", "total")]), r.total.counts()),
      ],
    );
    out.histogram(
      "moqtail_first_object_seconds",
      "Time from a subscription's creation to its first object written.",
      &[(String::new(), r.first_object.counts())],
    );
  }

  out.one(
    "moqtail_qlog_events_dropped_total",
    "counter",
    "qlog events dropped because the file writer fell behind.",
    moqtail::transport::qlog::dropped() as f64,
  );
}

fn counted(map: &std::sync::Mutex<BTreeMap<&'static str, u64>>, key: &str) -> Vec<(String, f64)> {
  map
    .lock()
    .unwrap()
    .iter()
    .map(|(k, n)| (label(&[(key, k)]), *n as f64))
    .collect()
}

/// Resident memory, from `/proc` on Linux; `None` elsewhere.
fn resident_memory_bytes() -> Option<u64> {
  let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
  let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
  Some(pages * 4096)
}

/// Open file descriptors, from `/proc` on Linux; `None` elsewhere.
fn open_fds() -> Option<usize> {
  Some(std::fs::read_dir("/proc/self/fd").ok()?.count())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn histograms_are_cumulative_and_filtered_families_are_left_out() {
    let mut out = Exposition::new(Some("seconds".into()));
    let mut counts = [0; BUCKETS];
    counts[0] = 2;
    counts[3] = 1;
    out.histogram(
      "x_seconds",
      "help",
      &[(label(&[("stage", "total")]), counts)],
    );
    out.one("other_total", "counter", "help", 1.0);
    let text = out.text;
    assert!(text.contains("x_seconds_bucket{stage=\"total\",le=\"0.00001\"} 2"));
    assert!(text.contains("x_seconds_bucket{stage=\"total\",le=\"0.00008\"} 3"));
    assert!(text.contains("x_seconds_bucket{stage=\"total\",le=\"+Inf\"} 3"));
    assert!(text.contains("x_seconds_count{stage=\"total\"} 3"));
    assert!(!text.contains("other_total"));
  }

  #[test]
  fn label_values_are_escaped() {
    assert_eq!(label(&[("v", "a\"b")]), "v=\"a\\\"b\"");
  }
}
