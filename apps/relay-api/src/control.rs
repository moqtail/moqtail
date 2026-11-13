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

//! Counts control messages and follows each request to its response.
//!
//! A request and its response travel on the same request stream in opposite
//! directions, so a request is matched to its answer by connection and stream.
//! A request the relay sent goes upstream, to a publisher or a cluster peer; a
//! request the relay received comes from downstream.

use crate::zones::Zones;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::transport::observe::ControlObserver;
use moqtail_probe::stats::{BUCKETS, Histogram};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Request durations kept per upstream source for its percentiles.
const DURATIONS_KEPT: usize = 256;

/// Control messages of one connection.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MessageCounts {
  pub received: u64,
  pub sent: u64,
  pub parse_errors: u64,
}

/// Requests of one connection, in either direction.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RequestCounts {
  pub in_flight: u64,
  pub ok: u64,
  pub error: u64,
}

/// What the tracker knows about one connection.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ConnectionCounts {
  pub control_messages: MessageCounts,
  pub requests: RequestCounts,
}

/// Results of one request type in one direction, relay-wide.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ResultCounts {
  pub ok: u64,
  /// By error code.
  pub error: BTreeMap<u64, u64>,
}

/// The last request a source refused.
#[derive(Debug, Clone, Serialize)]
pub struct LastError {
  pub at_ms: u64,
  pub request: &'static str,
  pub code: u64,
  pub reason: String,
}

/// Requests the relay sent to one source: a publisher or a cluster peer.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpstreamCounts {
  pub in_flight: u64,
  pub ok: u64,
  pub failed: u64,
  pub timed_out: u64,
  pub last_error: Option<LastError>,
  /// Milliseconds from the request to its answer.
  pub request_ms: Option<Percentiles>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Percentiles {
  pub p50: f64,
  pub p95: f64,
}

struct Pending {
  kind: &'static str,
  /// The namespace the request is about, as `a/b`; kept only when zones are counted.
  namespace: Option<String>,
  /// True when the relay sent the request, so it went upstream.
  upstream: bool,
  started: Instant,
}

#[derive(Default)]
struct Upstream {
  ok: u64,
  failed: u64,
  timed_out: u64,
  last_error: Option<LastError>,
  durations_ms: VecDeque<f64>,
}

#[derive(Default)]
struct Inner {
  connections: HashMap<usize, ConnectionCounts>,
  pending: HashMap<(usize, u64), Pending>,
  /// Keyed by request type and direction (`upstream` or `downstream`).
  results: BTreeMap<(&'static str, &'static str), ResultCounts>,
  /// Time from request to answer, by request type and direction.
  durations: BTreeMap<(&'static str, &'static str), Histogram>,
  upstreams: HashMap<usize, Upstream>,
  /// Relay-wide totals, kept when a connection closes.
  totals: Totals,
}

/// Relay-wide counts since the relay started.
#[derive(Debug, Clone, Default)]
pub struct Totals {
  /// By direction (`sent`, `received`) and message type.
  pub messages: BTreeMap<(&'static str, &'static str), u64>,
  pub parse_errors: u64,
  pub upstream_ok: u64,
  pub upstream_failed: u64,
  pub upstream_timed_out: u64,
}

pub(crate) struct ControlTracker {
  inner: Mutex<Inner>,
  /// An upstream request unanswered for this long counts as timed out.
  upstream_timeout: Duration,
  zones: Arc<Zones>,
}

impl ControlTracker {
  pub fn new(upstream_timeout: Duration, zones: Arc<Zones>) -> Self {
    ControlTracker {
      inner: Mutex::new(Inner::default()),
      upstream_timeout,
      zones,
    }
  }

  /// The counts of an open connection.
  pub fn connection(&self, conn_id: u64) -> ConnectionCounts {
    let inner = self.inner.lock().unwrap();
    let mut counts = inner
      .connections
      .get(&(conn_id as usize))
      .cloned()
      .unwrap_or_default();
    counts.requests.in_flight = inner
      .pending
      .keys()
      .filter(|(conn, _)| *conn as u64 == conn_id)
      .count() as u64;
    counts
  }

  /// Forgets a closed connection and returns what it had counted.
  pub fn connection_closed(&self, conn_id: u64) -> ConnectionCounts {
    let mut inner = self.inner.lock().unwrap();
    let conn = conn_id as usize;
    inner.pending.retain(|(c, _), _| *c != conn);
    inner.upstreams.remove(&conn);
    inner.connections.remove(&conn).unwrap_or_default()
  }

  /// The requests the relay sent to `conn_id`, with overdue ones counted as timed out.
  pub fn upstream(&self, conn_id: u64) -> Option<UpstreamCounts> {
    let mut inner = self.inner.lock().unwrap();
    let conn = conn_id as usize;
    let timeout = self.upstream_timeout;
    let overdue: Vec<(usize, u64)> = inner
      .pending
      .iter()
      .filter(|((c, _), p)| *c == conn && p.upstream && p.started.elapsed() > timeout)
      .map(|(key, _)| *key)
      .collect();
    for key in &overdue {
      inner.pending.remove(key);
    }
    let in_flight = inner
      .pending
      .iter()
      .filter(|((c, _), p)| *c == conn && p.upstream)
      .count() as u64;
    if !overdue.is_empty() {
      inner.upstreams.entry(conn).or_default().timed_out += overdue.len() as u64;
      inner.totals.upstream_timed_out += overdue.len() as u64;
    }
    let upstream = inner.upstreams.get(&conn)?;
    let mut durations: Vec<f64> = upstream.durations_ms.iter().copied().collect();
    durations.sort_by(f64::total_cmp);
    Some(UpstreamCounts {
      in_flight,
      ok: upstream.ok,
      failed: upstream.failed,
      timed_out: upstream.timed_out,
      last_error: upstream.last_error.clone(),
      request_ms: (!durations.is_empty()).then(|| Percentiles {
        p50: nearest_rank(&durations, 0.50),
        p95: nearest_rank(&durations, 0.95),
      }),
    })
  }

  /// Relay-wide totals since the relay started.
  pub fn totals(&self) -> Totals {
    self.inner.lock().unwrap().totals.clone()
  }

  /// Requests waiting for an answer, in either direction.
  pub fn in_flight(&self) -> usize {
    self.inner.lock().unwrap().pending.len()
  }

  /// Request results by request type and direction.
  pub fn results_by_kind(&self) -> Vec<(&'static str, &'static str, ResultCounts)> {
    let inner = self.inner.lock().unwrap();
    inner
      .results
      .iter()
      .map(|((kind, direction), counts)| (*kind, *direction, counts.clone()))
      .collect()
  }

  /// Request durations by request type and direction, as histogram bucket counts.
  pub fn durations(&self) -> Vec<(&'static str, &'static str, [u64; BUCKETS])> {
    let inner = self.inner.lock().unwrap();
    inner
      .durations
      .iter()
      .map(|((kind, direction), h)| (*kind, *direction, h.counts()))
      .collect()
  }

  /// Relay-wide request results, keyed by `type/direction`.
  pub fn results(&self) -> BTreeMap<String, ResultCounts> {
    let inner = self.inner.lock().unwrap();
    inner
      .results
      .iter()
      .map(|((kind, direction), counts)| (format!("{kind}/{direction}"), counts.clone()))
      .collect()
  }
}

impl ControlObserver for ControlTracker {
  fn control_message(&self, conn: usize, stream_id: u64, sent: bool, message: &ControlMessage) {
    let mut inner = self.inner.lock().unwrap();
    let counts = inner.connections.entry(conn).or_default();
    if sent {
      counts.control_messages.sent += 1;
    } else {
      counts.control_messages.received += 1;
    }
    let direction = if sent { "sent" } else { "received" };
    *inner
      .totals
      .messages
      .entry((direction, message_name(message)))
      .or_default() += 1;

    if let Some(kind) = request_kind(message) {
      let namespace = (!self.zones.is_empty())
        .then(|| request_namespace(message))
        .flatten();
      inner.pending.insert(
        (conn, stream_id),
        Pending {
          kind,
          namespace,
          upstream: sent,
          started: Instant::now(),
        },
      );
      return;
    }
    let Some(outcome) = response_outcome(message) else {
      return;
    };
    // A response travels the other way from its request.
    let answers_request = inner
      .pending
      .get(&(conn, stream_id))
      .is_some_and(|p| p.upstream != sent);
    if !answers_request {
      return;
    }
    let pending = inner.pending.remove(&(conn, stream_id)).unwrap();
    let direction = if pending.upstream {
      "upstream"
    } else {
      "downstream"
    };
    inner
      .durations
      .entry((pending.kind, direction))
      .or_default()
      .record(pending.started.elapsed());
    let result = inner.results.entry((pending.kind, direction)).or_default();
    match &outcome {
      Outcome::Ok => result.ok += 1,
      Outcome::Error { code, .. } => *result.error.entry(*code).or_default() += 1,
    }
    if let Some(namespace) = &pending.namespace {
      self
        .zones
        .request_answered(namespace, matches!(outcome, Outcome::Ok));
    }
    let requests = &mut inner.connections.entry(conn).or_default().requests;
    match outcome {
      Outcome::Ok => requests.ok += 1,
      Outcome::Error { .. } => requests.error += 1,
    }
    if pending.upstream {
      match outcome {
        Outcome::Ok => inner.totals.upstream_ok += 1,
        Outcome::Error { .. } => inner.totals.upstream_failed += 1,
      }
      let upstream = inner.upstreams.entry(conn).or_default();
      if upstream.durations_ms.len() == DURATIONS_KEPT {
        upstream.durations_ms.pop_front();
      }
      upstream
        .durations_ms
        .push_back(pending.started.elapsed().as_secs_f64() * 1000.0);
      match outcome {
        Outcome::Ok => upstream.ok += 1,
        Outcome::Error { code, reason } => {
          upstream.failed += 1;
          upstream.last_error = Some(LastError {
            at_ms: SystemTime::now()
              .duration_since(UNIX_EPOCH)
              .map_or(0, |d| d.as_millis() as u64),
            request: pending.kind,
            code,
            reason,
          });
        }
      }
    }
  }

  fn parse_error(&self, conn: usize, _stream_id: u64) {
    let mut inner = self.inner.lock().unwrap();
    inner.totals.parse_errors += 1;
    inner
      .connections
      .entry(conn)
      .or_default()
      .control_messages
      .parse_errors += 1;
  }
}

enum Outcome {
  Ok,
  Error { code: u64, reason: String },
}

/// The message's type as a metric label.
fn message_name(message: &ControlMessage) -> &'static str {
  match message {
    ControlMessage::Setup(_) => "setup",
    ControlMessage::Goaway(_) => "goaway",
    ControlMessage::RequestOk(_) => "request_ok",
    ControlMessage::RequestError(_) => "request_error",
    ControlMessage::Subscribe(_) => "subscribe",
    ControlMessage::SubscribeOk(_) => "subscribe_ok",
    ControlMessage::RequestUpdate(_) => "request_update",
    ControlMessage::Publish(_) => "publish",
    ControlMessage::PublishDone(_) => "publish_done",
    ControlMessage::Fetch(_) => "fetch",
    ControlMessage::FetchOk(_) => "fetch_ok",
    ControlMessage::TrackStatus(_) => "track_status",
    ControlMessage::PublishNamespace(_) => "publish_namespace",
    ControlMessage::Namespace(_) => "namespace",
    ControlMessage::NamespaceDone(_) => "namespace_done",
    ControlMessage::SubscribeNamespace(_) => "subscribe_namespace",
    ControlMessage::SubscribeTracks(_) => "subscribe_tracks",
    ControlMessage::PublishSkipped(_) => "publish_skipped",
  }
}

/// The request type a message opens, or `None` for anything else.
fn request_kind(message: &ControlMessage) -> Option<&'static str> {
  Some(match message {
    ControlMessage::Subscribe(_) => "subscribe",
    ControlMessage::Fetch(_) => "fetch",
    ControlMessage::TrackStatus(_) => "track_status",
    ControlMessage::Publish(_) => "publish",
    ControlMessage::PublishNamespace(_) => "publish_namespace",
    ControlMessage::SubscribeNamespace(_) => "subscribe_namespace",
    ControlMessage::SubscribeTracks(_) => "subscribe_tracks",
    _ => return None,
  })
}

/// The namespace a request is about, as `a/b`.
fn request_namespace(message: &ControlMessage) -> Option<String> {
  let tuple = match message {
    ControlMessage::Subscribe(m) => &m.track_namespace,
    ControlMessage::Fetch(m) => &m.track_namespace,
    ControlMessage::TrackStatus(m) => &m.track_namespace,
    ControlMessage::Publish(m) => &m.track_namespace,
    ControlMessage::PublishNamespace(m) => &m.track_namespace,
    ControlMessage::SubscribeNamespace(m) => &m.track_namespace_prefix,
    ControlMessage::SubscribeTracks(m) => &m.track_namespace_prefix,
    _ => return None,
  };
  let fields: Vec<String> = tuple.fields.iter().map(|f| f.to_string()).collect();
  Some(fields.join("/"))
}

/// How a message answers a request, or `None` when it answers none.
fn response_outcome(message: &ControlMessage) -> Option<Outcome> {
  Some(match message {
    ControlMessage::SubscribeOk(_) | ControlMessage::FetchOk(_) | ControlMessage::RequestOk(_) => {
      Outcome::Ok
    }
    ControlMessage::RequestError(e) => Outcome::Error {
      code: u64::from(e.error_code),
      reason: e.reason_phrase.as_str().to_string(),
    },
    _ => return None,
  })
}

/// The nearest-rank percentile of sorted, non-empty values.
fn nearest_rank(sorted: &[f64], q: f64) -> f64 {
  let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
  sorted[rank - 1]
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::common::reason_phrase::ReasonPhrase;
  use moqtail::model::common::tuple::{Tuple, TupleField};
  use moqtail::model::control::request_error::RequestError;
  use moqtail::model::control::request_ok::RequestOk;
  use moqtail::model::control::subscribe::Subscribe;
  use moqtail::model::error::RequestErrorCode;

  fn subscribe() -> ControlMessage {
    ControlMessage::Subscribe(Box::new(Subscribe {
      request_id: 0,
      track_namespace: Tuple::from_utf8_path("live"),
      track_name: TupleField::from_utf8("video"),
      subscribe_parameters: vec![],
    }))
  }

  fn refused() -> ControlMessage {
    ControlMessage::RequestError(Box::new(RequestError::new(
      RequestErrorCode::DoesNotExist,
      0,
      ReasonPhrase::try_new("no such track".into()).unwrap(),
    )))
  }

  fn ok() -> ControlMessage {
    ControlMessage::RequestOk(Box::new(RequestOk {
      parameters: vec![],
      track_properties: vec![],
    }))
  }

  #[test]
  fn matches_answers_to_requests_by_stream_and_direction() {
    let tracker = ControlTracker::new(Duration::from_secs(5), Arc::new(Zones::new(&[])));
    // Downstream: a subscriber asks, the relay refuses.
    tracker.control_message(1, 4, false, &subscribe());
    assert_eq!(tracker.connection(1).requests.in_flight, 1);
    tracker.control_message(1, 4, true, &refused());
    let counts = tracker.connection(1);
    assert_eq!(counts.requests.error, 1);
    assert_eq!(counts.control_messages.received, 1);
    assert_eq!(counts.control_messages.sent, 1);

    // Upstream: the relay asks a publisher, which answers on the same stream.
    tracker.control_message(2, 8, true, &subscribe());
    tracker.control_message(2, 8, false, &ok());
    // A second answer on the stream answers nothing.
    tracker.control_message(2, 8, false, &ok());
    let upstream = tracker.upstream(2).unwrap();
    assert_eq!(
      (upstream.ok, upstream.failed, upstream.in_flight),
      (1, 0, 0)
    );
    assert!(upstream.request_ms.is_some());

    let results = tracker.results();
    assert_eq!(results["subscribe/downstream"].error[&0x10], 1);
    assert_eq!(results["subscribe/upstream"].ok, 1);
  }

  #[test]
  fn an_upstream_request_overdue_counts_as_timed_out() {
    let tracker = ControlTracker::new(Duration::ZERO, Arc::new(Zones::new(&[])));
    tracker.control_message(3, 4, true, &subscribe());
    std::thread::sleep(Duration::from_millis(2));
    tracker.control_message(3, 8, false, &ok()); // answers nothing; creates the source
    let upstream = tracker.upstream(3).unwrap();
    assert_eq!((upstream.timed_out, upstream.in_flight), (1, 0));
  }

  #[test]
  fn a_refusal_from_upstream_is_the_last_error() {
    let tracker = ControlTracker::new(Duration::from_secs(5), Arc::new(Zones::new(&[])));
    tracker.control_message(5, 4, true, &subscribe());
    tracker.control_message(5, 4, false, &refused());
    let upstream = tracker.upstream(5).unwrap();
    assert_eq!(upstream.failed, 1);
    assert_eq!(upstream.last_error.unwrap().reason, "no such track");
    tracker.connection_closed(5);
    assert!(tracker.upstream(5).is_none());
  }
}
