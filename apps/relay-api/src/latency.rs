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

//! Object processing time: the histograms the relay core records into, handed out
//! per subscription; copies of them over time, for percentiles over a window; the
//! slowlog; and `/api/v1/latency`.

use crate::ApiState;
use crate::error::ApiError;
use crate::list::parse_query;
use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use moqtail_probe::stats::{
  BUCKETS, ObjectTimes, RelayStats, SlowObjects, SubscriptionStats, TrackStats,
  bucket_bounds_micros,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How often the histograms are copied, and how many copies are kept (5 minutes).
const COPY_EVERY: Duration = Duration::from_secs(10);
const COPIES_KEPT: usize = 31;
/// Subscriptions report their last minute.
const SUBSCRIPTION_WINDOW: Duration = Duration::from_secs(60);
/// Entries in the slowest-tracks and slowest-subscriptions lists.
const SLOWEST: usize = 10;

type Counts = [u64; BUCKETS];

/// How finely processing time is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LatencyDetail {
  /// Relay-wide histograms per stage only.
  Relay,
  /// Plus one histogram per track.
  Track,
  /// Plus one histogram per subscription.
  Subscription,
}

impl LatencyDetail {
  pub fn parse(name: &str) -> anyhow::Result<Self> {
    Ok(match name {
      "relay" => LatencyDetail::Relay,
      "track" => LatencyDetail::Track,
      "subscription" => LatencyDetail::Subscription,
      other => anyhow::bail!("latency_detail is relay, track or subscription, not `{other}`"),
    })
  }
}

/// The slowlog's settings.
#[derive(Debug, Clone, Copy)]
pub struct SlowlogConfig {
  pub threshold: Duration,
  pub first_object_threshold: Duration,
  pub size: usize,
}

/// A copy of the histograms at one moment.
struct Copy {
  at: Instant,
  /// ingest, queue, write, total, first object.
  relay: [Counts; 5],
  tracks: HashMap<u64, Counts>,
}

pub(crate) struct Latency {
  pub relay: Arc<RelayStats>,
  detail: LatencyDetail,
  sample: u64,
  slowlog: Arc<Slowlog>,
  /// Live tracks' histograms, by track id. A track's entry goes when its last
  /// subscription does.
  tracks: Mutex<HashMap<u64, Weak<TrackStats>>>,
  /// Live subscriptions' handles.
  subscriptions: Mutex<Vec<Weak<SubscriptionStats>>>,
  copies: Mutex<VecDeque<Copy>>,
  /// Each subscription's histogram as it was about a minute ago.
  subscription_copies: Mutex<HashMap<(u64, u64), (Instant, Counts)>>,
}

impl Latency {
  pub fn new(detail: LatencyDetail, sample: u64, slowlog: SlowlogConfig) -> Self {
    Latency {
      relay: Arc::new(RelayStats::default()),
      detail,
      sample,
      slowlog: Arc::new(Slowlog::new(slowlog)),
      tracks: Mutex::new(HashMap::new()),
      subscriptions: Mutex::new(Vec::new()),
      copies: Mutex::new(VecDeque::new()),
      subscription_copies: Mutex::new(HashMap::new()),
    }
  }

  /// A new subscription's handle, sharing its track's histogram with the other
  /// subscriptions of the track.
  pub fn subscription_stats(
    &self,
    track_id: u64,
    conn_id: u64,
    request_id: u64,
  ) -> Arc<SubscriptionStats> {
    let track = (self.detail >= LatencyDetail::Track).then(|| {
      let mut tracks = self.tracks.lock().unwrap();
      tracks.retain(|_, t| t.strong_count() > 0);
      match tracks.get(&track_id).and_then(Weak::upgrade) {
        Some(existing) => existing,
        None => {
          let created = Arc::new(TrackStats::default());
          tracks.insert(track_id, Arc::downgrade(&created));
          created
        }
      }
    });
    let slowlog: Arc<dyn SlowObjects> = self.slowlog.clone();
    let stats = Arc::new(SubscriptionStats::new(
      self.relay.clone(),
      track,
      self.detail == LatencyDetail::Subscription,
      self.sample,
      Some(slowlog),
      track_id,
      conn_id,
      request_id,
    ));
    let mut subscriptions = self.subscriptions.lock().unwrap();
    subscriptions.retain(|s| s.strong_count() > 0);
    subscriptions.push(Arc::downgrade(&stats));
    stats
  }

  fn live_tracks(&self) -> Vec<(u64, Arc<TrackStats>)> {
    let tracks = self.tracks.lock().unwrap();
    tracks
      .iter()
      .filter_map(|(id, t)| Some((*id, t.upgrade()?)))
      .collect()
  }

  fn live_subscriptions(&self) -> Vec<Arc<SubscriptionStats>> {
    let subscriptions = self.subscriptions.lock().unwrap();
    subscriptions.iter().filter_map(Weak::upgrade).collect()
  }

  fn relay_counts(&self) -> [Counts; 5] {
    let r = &self.relay;
    [
      r.ingest.counts(),
      r.queue.counts(),
      r.write.counts(),
      r.total.counts(),
      r.first_object.counts(),
    ]
  }

  /// Copies the histograms; called every [`COPY_EVERY`] on the API's runtime.
  pub fn take_copy(&self) {
    let now = Instant::now();
    let copy = Copy {
      at: now,
      relay: self.relay_counts(),
      tracks: self
        .live_tracks()
        .into_iter()
        .map(|(id, t)| (id, t.total.counts()))
        .collect(),
    };
    let mut copies = self.copies.lock().unwrap();
    if copies.len() == COPIES_KEPT {
      copies.pop_front();
    }
    copies.push_back(copy);
    drop(copies);

    let live = self.live_subscriptions();
    let mut subscription_copies = self.subscription_copies.lock().unwrap();
    subscription_copies.retain(|key, _| live.iter().any(|s| (s.conn_id, s.request_id) == *key));
    for s in live {
      let Some(own) = &s.own else { continue };
      let entry = subscription_copies
        .entry((s.conn_id, s.request_id))
        .or_insert((now, [0; BUCKETS]));
      if now - entry.0 >= SUBSCRIPTION_WINDOW {
        *entry = (now, own.counts());
      }
    }
  }

  /// The copy that starts `window`, and how long ago it was taken. `None` for
  /// "since start". When the relay is younger than the window, the oldest copy.
  fn window_start(
    &self,
    window: Option<Duration>,
  ) -> Option<(Duration, [Counts; 5], HashMap<u64, Counts>)> {
    let window = window?;
    let copies = self.copies.lock().unwrap();
    let now = Instant::now();
    let start = copies
      .iter()
      .rev()
      .find(|c| now - c.at >= window)
      .or_else(|| copies.front())?;
    Some((now - start.at, start.relay, start.tracks.clone()))
  }

  /// Percentiles of one track over the last minute, if it is kept.
  pub fn track_summary(&self, track_id: u64) -> Option<Summary> {
    let track = self.tracks.lock().unwrap().get(&track_id)?.upgrade()?;
    let now = track.total.counts();
    let before = self
      .window_start(Some(SUBSCRIPTION_WINDOW))
      .and_then(|(_, _, tracks)| tracks.get(&track_id).copied())
      .unwrap_or([0; BUCKETS]);
    Some(summarize(&minus(&now, &before)))
  }

  /// Percentiles of one subscription over about the last minute, if it is kept.
  pub fn subscription_summary(&self, conn_id: u64, request_id: u64) -> Option<Summary> {
    let stats = self
      .live_subscriptions()
      .into_iter()
      .find(|s| (s.conn_id, s.request_id) == (conn_id, request_id))?;
    let now = stats.own.as_ref()?.counts();
    let before = self
      .subscription_copies
      .lock()
      .unwrap()
      .get(&(conn_id, request_id))
      .map_or([0; BUCKETS], |(_, c)| *c);
    Some(summarize(&minus(&now, &before)))
  }
}

/// Runs [`Latency::take_copy`] every ten seconds, for as long as the API runs.
pub(crate) async fn copy_periodically(latency: Arc<Latency>) {
  let mut tick = tokio::time::interval(COPY_EVERY);
  loop {
    tick.tick().await;
    latency.take_copy();
  }
}

fn minus(now: &Counts, before: &Counts) -> Counts {
  std::array::from_fn(|i| now[i].saturating_sub(before[i]))
}

/// Percentiles from bucket counts. A value is reported as its bucket's upper bound,
/// so it is within a factor of two above the real one.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
  pub count: u64,
  pub p50: Option<f64>,
  pub p95: Option<f64>,
  pub p99: Option<f64>,
  pub max: Option<f64>,
}

pub fn summarize(counts: &Counts) -> Summary {
  let bounds = bucket_bounds_micros();
  // The last bucket has no upper bound; report twice the one below it.
  let bound_ms = |i: usize| -> f64 {
    let micros = if i == BUCKETS - 1 {
      bounds[BUCKETS - 2] * 2
    } else {
      bounds[i]
    };
    micros as f64 / 1000.0
  };
  let count: u64 = counts.iter().sum();
  let rank = |q: f64| -> Option<f64> {
    if count == 0 {
      return None;
    }
    let target = ((q * count as f64).ceil() as u64).max(1);
    let mut seen = 0;
    for (i, c) in counts.iter().enumerate() {
      seen += c;
      if seen >= target {
        return Some(bound_ms(i));
      }
    }
    None
  };
  Summary {
    count,
    p50: rank(0.50),
    p95: rank(0.95),
    p99: rank(0.99),
    max: counts.iter().rposition(|c| *c > 0).map(bound_ms),
  }
}

/// Deliveries and subscriptions over their threshold, newest last.
#[derive(Debug)]
pub(crate) struct Slowlog {
  config: SlowlogConfig,
  entries: Mutex<VecDeque<SlowEntry>>,
  /// Entries dropped because the list was busy at that moment.
  dropped: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlowEntry {
  at_ms: u64,
  /// `object` for a slow delivery, `first_object` for a slow subscription start.
  kind: &'static str,
  total_ms: f64,
  #[serde(skip_serializing_if = "Option::is_none")]
  stages_ms: Option<Stages>,
  track_id: u64,
  conn_id: u64,
  request_id: u64,
  #[serde(skip_serializing_if = "Option::is_none")]
  location: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Stages {
  ingest: f64,
  queue: f64,
  write: f64,
}

impl Slowlog {
  fn new(config: SlowlogConfig) -> Self {
    Slowlog {
      config,
      entries: Mutex::new(VecDeque::new()),
      dropped: AtomicU64::new(0),
    }
  }

  /// Adds an entry without waiting: when another delivery holds the list, this one
  /// is dropped and counted rather than slowing the delivery down.
  fn push(&self, entry: SlowEntry) {
    if self.config.size == 0 {
      return;
    }
    match self.entries.try_lock() {
      Ok(mut entries) => {
        if entries.len() == self.config.size {
          entries.pop_front();
        }
        entries.push_back(entry);
      }
      Err(_) => {
        self.dropped.fetch_add(1, Ordering::Relaxed);
      }
    }
  }
}

fn ms(d: Duration) -> f64 {
  d.as_secs_f64() * 1000.0
}

fn now_ms() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| d.as_millis() as u64)
}

impl SlowObjects for Slowlog {
  fn threshold(&self) -> Duration {
    self.config.threshold
  }

  fn slow_object(&self, stats: &SubscriptionStats, times: ObjectTimes, group: u64, object: u64) {
    self.push(SlowEntry {
      at_ms: now_ms(),
      kind: "object",
      total_ms: ms(times.written - times.arrived),
      stages_ms: Some(Stages {
        ingest: ms(times.queued - times.arrived),
        queue: ms(times.picked - times.queued),
        write: ms(times.written - times.picked),
      }),
      track_id: stats.track_id,
      conn_id: stats.conn_id,
      request_id: stats.request_id,
      location: Some(format!("{group}:{object}")),
    });
  }

  fn first_object_threshold(&self) -> Duration {
    self.config.first_object_threshold
  }

  fn slow_first_object(&self, stats: &SubscriptionStats, waited: Duration) {
    self.push(SlowEntry {
      at_ms: now_ms(),
      kind: "first_object",
      total_ms: ms(waited),
      stages_ms: None,
      track_id: stats.track_id,
      conn_id: stats.conn_id,
      request_id: stats.request_id,
      location: None,
    });
  }
}

#[derive(Debug, Default, Deserialize)]
pub struct LatencyQuery {
  /// `10s`, `60s` (the default), `5m` or `start`.
  window: Option<String>,
}

#[derive(Serialize)]
pub struct LatencyReport {
  window: String,
  /// How many seconds the numbers actually cover; less than the window while the
  /// relay is younger than it.
  covered_s: Option<f64>,
  stages_ms: StagesSummary,
  /// From a subscription's creation to its first object written.
  first_object_ms: Summary,
  slowest_tracks: Vec<SlowTrack>,
  slowest_subscriptions: Vec<SlowSubscription>,
}

#[derive(Serialize)]
struct StagesSummary {
  ingest: Summary,
  queue: Summary,
  write: Summary,
  total: Summary,
}

#[derive(Serialize)]
struct SlowTrack {
  track_id: u64,
  track: Option<String>,
  #[serde(flatten)]
  summary: Summary,
}

#[derive(Serialize)]
struct SlowSubscription {
  conn_id: u64,
  request_id: u64,
  track_id: u64,
  #[serde(flatten)]
  summary: Summary,
}

fn latency_of(state: &ApiState) -> Result<&Arc<Latency>, ApiError> {
  state.probe.latency.as_ref().ok_or_else(|| {
    ApiError::new(StatusCode::NOT_FOUND, "feature_off")
      .detail("add `latency` to api_features to turn this endpoint on")
  })
}

pub async fn report(
  State(state): State<Arc<ApiState>>,
  query: Result<Query<LatencyQuery>, QueryRejection>,
) -> Result<Json<LatencyReport>, ApiError> {
  let query = parse_query(query)?;
  let latency = latency_of(&state)?;
  let window_name = query.window.unwrap_or_else(|| "60s".into());
  let window = match window_name.as_str() {
    "10s" => Some(Duration::from_secs(10)),
    "60s" => Some(Duration::from_secs(60)),
    "5m" => Some(Duration::from_secs(300)),
    "start" => None,
    _ => {
      return Err(
        ApiError::new(StatusCode::BAD_REQUEST, "unknown_window")
          .detail("window is one of 10s, 60s, 5m, start"),
      );
    }
  };

  let now = latency.relay_counts();
  let start = latency.window_start(window);
  let (covered, before, track_before) = match start {
    Some((age, relay, tracks)) => (Some(age.as_secs_f64()), relay, tracks),
    None => (None, [[0; BUCKETS]; 5], HashMap::new()),
  };
  let stage = |i: usize| summarize(&minus(&now[i], &before[i]));

  let names: HashMap<u64, String> = state
    .tracks()
    .await
    .iter()
    .map(|t| (t.track_id, format!("{} : {}", t.namespace, t.name)))
    .collect();
  let mut slowest_tracks: Vec<SlowTrack> = latency
    .live_tracks()
    .into_iter()
    .map(|(id, t)| {
      let before = track_before.get(&id).copied().unwrap_or([0; BUCKETS]);
      SlowTrack {
        track_id: id,
        track: names.get(&id).cloned(),
        summary: summarize(&minus(&t.total.counts(), &before)),
      }
    })
    .filter(|t| t.summary.count > 0)
    .collect();
  slowest_tracks.sort_by(|a, b| b.summary.p99.partial_cmp(&a.summary.p99).unwrap());
  slowest_tracks.truncate(SLOWEST);

  let mut slowest_subscriptions: Vec<SlowSubscription> = latency
    .live_subscriptions()
    .into_iter()
    .filter_map(|s| {
      let summary = latency.subscription_summary(s.conn_id, s.request_id)?;
      (summary.count > 0).then_some(SlowSubscription {
        conn_id: s.conn_id,
        request_id: s.request_id,
        track_id: s.track_id,
        summary,
      })
    })
    .collect();
  slowest_subscriptions.sort_by(|a, b| b.summary.p99.partial_cmp(&a.summary.p99).unwrap());
  slowest_subscriptions.truncate(SLOWEST);

  Ok(Json(LatencyReport {
    window: window_name,
    covered_s: covered,
    stages_ms: StagesSummary {
      ingest: stage(0),
      queue: stage(1),
      write: stage(2),
      total: stage(3),
    },
    first_object_ms: stage(4),
    slowest_tracks,
    slowest_subscriptions,
  }))
}

#[derive(Serialize)]
pub struct SlowlogReport {
  threshold_ms: f64,
  first_object_threshold_ms: f64,
  /// Entries not kept because another delivery was adding one at that moment.
  dropped: u64,
  /// Newest first.
  entries: Vec<SlowEntry>,
}

pub async fn slowlog(State(state): State<Arc<ApiState>>) -> Result<Json<SlowlogReport>, ApiError> {
  let latency = latency_of(&state)?;
  let log = &latency.slowlog;
  let entries = log.entries.lock().unwrap().iter().rev().cloned().collect();
  Ok(Json(SlowlogReport {
    threshold_ms: ms(log.config.threshold),
    first_object_threshold_ms: ms(log.config.first_object_threshold),
    dropped: log.dropped.load(Ordering::Relaxed),
    entries,
  }))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn latency(detail: LatencyDetail) -> Latency {
    Latency::new(
      detail,
      1,
      SlowlogConfig {
        threshold: Duration::ZERO,
        first_object_threshold: Duration::from_secs(3600),
        size: 2,
      },
    )
  }

  #[test]
  fn subscriptions_of_a_track_share_its_histogram() {
    let latency = latency(LatencyDetail::Subscription);
    let a = latency.subscription_stats(7, 1, 0);
    let b = latency.subscription_stats(7, 2, 0);
    assert_eq!(latency.live_tracks().len(), 1);
    assert_eq!(latency.live_subscriptions().len(), 2);
    assert!(a.own.is_some());
    drop((a, b));
    assert!(
      latency.live_tracks().is_empty(),
      "the track goes with its subscriptions"
    );
    assert!(latency.live_subscriptions().is_empty());
  }

  #[test]
  fn relay_detail_keeps_no_track_or_subscription_histogram() {
    let latency = latency(LatencyDetail::Relay);
    let stats = latency.subscription_stats(7, 1, 0);
    assert!(stats.own.is_none());
    assert!(latency.live_tracks().is_empty());
  }

  #[test]
  fn summary_reports_bucket_bounds() {
    let mut counts = [0; BUCKETS];
    counts[0] = 98; // up to 10 µs
    counts[10] = 2; // up to 10.24 ms
    let s = summarize(&counts);
    assert_eq!(s.count, 100);
    assert_eq!(s.p50, Some(0.01));
    assert_eq!(s.p99, Some(10.24));
    assert_eq!(s.max, Some(10.24));
    assert_eq!(summarize(&[0; BUCKETS]).p50, None);
  }

  #[test]
  fn slow_deliveries_are_kept_newest_last_up_to_the_size() {
    let latency = latency(LatencyDetail::Track);
    let stats = latency.subscription_stats(7, 1, 0);
    let t = Instant::now();
    for object in 0..3 {
      stats.object_written(Some(t), t, t, 4, object);
    }
    let entries = latency.slowlog.entries.lock().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries.back().unwrap().location.as_deref(), Some("4:2"));
  }

  #[test]
  fn windows_subtract_the_copy_taken_at_their_start() {
    let latency = latency(LatencyDetail::Track);
    let stats = latency.subscription_stats(7, 1, 0);
    let t = Instant::now();
    stats.object_written(Some(t), t, t, 0, 0);
    latency.take_copy();
    stats.object_written(Some(t), t, t, 0, 1);
    // With one copy younger than the window, the window starts at that copy.
    let (_, before, tracks) = latency.window_start(Some(Duration::from_secs(60))).unwrap();
    let total_now = latency.relay_counts()[3];
    assert_eq!(summarize(&minus(&total_now, &before[3])).count, 1);
    assert_eq!(tracks[&7].iter().sum::<u64>(), 1);
    assert_eq!(latency.track_summary(7).unwrap().count, 1);
  }
}
