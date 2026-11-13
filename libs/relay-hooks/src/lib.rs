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

//! The hooks between the relay core and its observability layer.
//!
//! There are three, one per direction of travel:
//!
//! - [`Inspect`]: the core implements it so the observability layer can read the
//!   relay's state as plain copies, without holding the core's locks.
//! - [`Actions`]: the core implements it so an operator can change the running
//!   relay, such as sending GOAWAY or replacing the log filter.
//! - [`Reporter`]: the observability layer implements it, and the core calls it as
//!   things happen, such as a connection closing or a request being refused.
//!
//! Everything that serves or exports this data lives in the observability crate,
//! `relay-api`, which the core never imports.

pub mod settings;
pub mod stats;

use serde::Serialize;
use stats::{RelayCounters, SubscriptionStats, ZoneCounters};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

/// A boxed future, so [`Inspect`] can be used as a trait object.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Read-only access to the relay's state.
///
/// Each method copies what it needs while holding the core's locks only for the
/// copy, then returns plain data. Callers never hold a core lock, so a slow reader
/// cannot stall delivery.
pub trait Inspect: Send + Sync {
  /// The relay's identity and lifecycle state.
  fn status(&self) -> RelayStatus;

  /// The effective configuration, with every secret left out.
  fn config(&self) -> serde_json::Value;

  /// Every connection that has completed SETUP and is still open.
  fn connections(&self) -> BoxFuture<'_, Vec<ConnectionInfo>>;

  /// Every track the relay knows.
  fn tracks(&self) -> BoxFuture<'_, Vec<TrackInfo>>;

  /// One track with its subscriptions and cache contents.
  fn track(&self, track_id: u64) -> BoxFuture<'_, Option<TrackDetail>>;

  /// Every SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS the relay holds.
  fn namespace_subscriptions(&self) -> BoxFuture<'_, Vec<NamespaceSubscriptionInfo>>;

  /// This relay's cluster identity and its links to other relays.
  fn cluster(&self) -> BoxFuture<'_, ClusterInfo>;
}

/// What an operator can change in the running relay, through the API's `actions`
/// feature. Each call returns once the relay has acted.
pub trait Actions: Send + Sync {
  /// Sends GOAWAY to one connection, asking it to move to another session within
  /// `timeout_ms`, and closes it with GOAWAY_TIMEOUT if it is still open then.
  /// `false` when no open connection has that id.
  fn goaway(&self, conn_id: u64, timeout_ms: u64) -> BoxFuture<'_, bool>;

  /// Closes one connection at once. `false` when no open connection has that id.
  fn close(&self, conn_id: u64) -> BoxFuture<'_, bool>;

  /// Refuses new requests from now on and sends GOAWAY to every connection,
  /// closing those still open after `timeout_ms`. The relay keeps running until it
  /// is stopped. Returns how many connections were told.
  fn drain(&self, timeout_ms: u64) -> BoxFuture<'_, usize>;

  /// The URI that GOAWAY sends clients to; `None` when GOAWAY carries none.
  fn redirect_uri(&self) -> BoxFuture<'_, Option<String>>;

  fn set_redirect_uri(&self, uri: Option<String>) -> BoxFuture<'_, ()>;

  /// The log filter in effect, such as `info,relay=debug`; `None` when the relay
  /// was started without a filter that can be changed.
  fn log_filter(&self) -> Option<String>;

  /// Replaces the log filter. The error says why a filter was not taken.
  fn set_log_filter(&self, filter: &str) -> Result<(), String>;
}

/// What the relay core reports as things happen. Every method has an empty default,
/// so the core can always call them, and [`NoReporter`] is the reporter when nothing
/// observes the relay.
pub trait Reporter: Send + Sync {
  /// True when [`Reporter::connection_closed`] keeps what it is given. The core builds a
  /// closing connection's summary only then.
  fn keeps_closed_connections(&self) -> bool {
    false
  }

  /// A connection closed. `connection` describes it as it was at that moment, with
  /// its close reason set.
  fn connection_closed(&self, _connection: ConnectionInfo) {}

  /// The handle a new subscription records its deliveries with; `None` when
  /// processing time is not measured, so the subscription records nothing.
  fn subscription_stats(
    &self,
    _track_id: u64,
    _conn_id: u64,
    _request_id: u64,
  ) -> Option<Arc<SubscriptionStats>> {
    None
  }

  /// The counters of the accounting zone a track's namespace (`a/b/c`) falls in;
  /// `None` when no zones are configured.
  fn zone(&self, _namespace: &str) -> Option<Arc<ZoneCounters>> {
    None
  }

  /// A connection failed before it became a session: `reason` is `handshake` or
  /// `setup`.
  fn connection_failed(&self, _reason: &'static str) {}

  /// A request was refused or shed because of a limit: `draining`,
  /// `max_active_requests` or `max_publish_streams`.
  fn limit_rejected(&self, _limit: &'static str) {}
}

/// The reporter that ignores everything.
pub struct NoReporter;

impl Reporter for NoReporter {}

static REPORTER: OnceLock<Arc<dyn Reporter>> = OnceLock::new();

/// Installs the reporter the relay core reports to, once, before the relay serves
/// anything. Later calls are ignored.
pub fn install(reporter: Arc<dyn Reporter>) {
  let _ = REPORTER.set(reporter);
}

static COUNTERS: OnceLock<Arc<RelayCounters>> = OnceLock::new();

/// Installs the relay-wide traffic counters that the delivery path adds to. Later
/// calls are ignored.
pub fn install_counters(counters: Arc<RelayCounters>) {
  let _ = COUNTERS.set(counters);
}

/// The installed traffic counters; `None` when none were installed, and the
/// delivery path then skips counting.
pub fn counters() -> Option<&'static RelayCounters> {
  COUNTERS.get().map(|c| c.as_ref())
}

/// The installed reporter, or [`NoReporter`] when none is.
pub fn reporter() -> &'static dyn Reporter {
  static NONE: NoReporter = NoReporter;
  REPORTER
    .get()
    .map_or(&NONE as &dyn Reporter, |p| p.as_ref())
}

/// The relay's identity and lifecycle state.
#[derive(Debug, Clone, Serialize)]
pub struct RelayStatus {
  /// Build version, for example `moqtail/0.18.0+9e8bf05f`.
  pub version: String,
  /// Process id.
  pub pid: u32,
  /// When the relay started, in milliseconds since the Unix epoch.
  pub started_at_ms: u64,
  /// True once the QUIC listeners accept connections.
  pub accepting: bool,
  /// True after GOAWAY was broadcast; new requests are rejected from then on.
  pub draining: bool,
}

/// Where a connection is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
  /// It has requests: announcements, publications or subscriptions.
  Active,
  /// It has no requests.
  Idle,
  /// The relay is draining and has asked it to move.
  Draining,
  /// It has closed; kept in the history of closed connections.
  Closed,
}

/// Who a connection is to the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
  /// A publisher or subscriber.
  Client,
  /// Another relay of the cluster.
  ClusterPeer,
}

/// One connection, copied out of the relay.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionInfo {
  pub conn_id: u64,
  pub state: ConnectionState,
  /// `webtransport` or `quic`.
  pub transport: String,
  pub remote: String,
  pub role: Role,
  /// When SETUP completed, in milliseconds since the Unix epoch.
  pub connected_at_ms: u64,
  /// The peer's implementation, from its SETUP, when it sent one.
  pub implementation: Option<String>,
  /// Every option of the peer's SETUP.
  pub setup_options: Vec<SetupOptionInfo>,
  /// Namespaces this peer announced, as `a/b/c`.
  pub announced_namespaces: Vec<String>,
  pub published_tracks: Vec<PublishedTrackInfo>,
  pub subscriptions: Vec<SubscriptionInfo>,
  /// The QUIC connection's statistics.
  pub quic: QuicStats,
  /// Set once the connection has closed.
  pub close: Option<CloseInfo>,
}

/// One SETUP option: a known name with a readable value, or the raw type number.
#[derive(Debug, Clone, Serialize)]
pub struct SetupOptionInfo {
  pub name: String,
  pub value: String,
}

/// A track a peer publishes to the relay.
#[derive(Debug, Clone, Serialize)]
pub struct PublishedTrackInfo {
  pub request_id: u64,
  /// `namespace : name`.
  pub track: String,
}

/// One subscription a peer holds, with its progress.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionInfo {
  pub request_id: u64,
  /// `namespace : name`.
  pub track: String,
  /// The relay's own id for the track, stable for the track's life.
  pub track_id: u64,
  /// The alias the subscriber knows the track by.
  pub track_alias: u64,
  pub forward: bool,
  pub subscriber_priority: u8,
  pub group_order: String,
  /// The location filter, as the relay understood it.
  pub filter: String,
  /// The first location the subscription delivers, as `group:object`.
  pub start: Option<String>,
  pub end_group: Option<u64>,
  /// The largest location sent so far.
  pub last_sent: Option<String>,
  /// The largest location received from upstream for this subscription.
  pub last_received: Option<String>,
  pub streams_opened: u64,
  /// Payload bytes waiting for this subscription's task.
  pub queued_bytes: u64,
}

/// QUIC-level statistics of a connection.
#[derive(Debug, Clone, Default, Serialize)]
pub struct QuicStats {
  pub rtt_ms: f64,
  pub cwnd: u64,
  pub congestion_events: u64,
  pub lost_packets: u64,
  pub lost_bytes: u64,
  pub sent_bytes: u64,
  pub received_bytes: u64,
  pub sent_datagrams: u64,
  pub received_datagrams: u64,
}

/// Why and when a connection closed.
#[derive(Debug, Clone, Serialize)]
pub struct CloseInfo {
  /// Milliseconds since the Unix epoch.
  pub at_ms: u64,
  /// `peer`, `relay`, or `network` (timeout or reset).
  pub by: String,
  /// The application or transport error code, when there was one.
  pub code: Option<u64>,
  pub reason: String,
}

/// One track, copied out of the relay.
#[derive(Debug, Clone, Serialize)]
pub struct TrackInfo {
  /// The relay's own id for the track, stable for the track's life.
  pub track_id: u64,
  /// The namespace as `a/b/c`.
  pub namespace: String,
  pub name: String,
  /// `publish` (a PUBLISH brought it), `subscribe` (a SUBSCRIBE reached a
  /// publisher), or `cache_only` (only cached objects remain).
  pub origin: String,
  /// `pending`, `confirmed` or `rejected`.
  pub status: String,
  pub publishers: Vec<TrackPublisher>,
  pub subscribers: usize,
  /// True while at least one subscriber wants objects forwarded.
  pub forwarding: bool,
  /// The largest location received, as `group:object`.
  pub largest_location: Option<String>,
  pub cached_groups: u64,
  /// True while the relay asks its publishers to forward objects.
  pub upstream_forward: bool,
}

/// A publisher of a track, and the alias it publishes it under.
#[derive(Debug, Clone, Serialize)]
pub struct TrackPublisher {
  pub conn_id: u64,
  pub alias: u64,
}

/// A track with its subscriptions and cache contents.
#[derive(Debug, Clone, Serialize)]
pub struct TrackDetail {
  #[serde(flatten)]
  pub track: TrackInfo,
  pub subscriptions: Vec<TrackSubscription>,
  /// The ids of the cached groups, ascending.
  pub cached_group_ids: Vec<u64>,
  /// Payload bytes held in the cache for this track.
  pub cached_bytes: u64,
}

/// A subscription to a track, and the connection holding it.
#[derive(Debug, Clone, Serialize)]
pub struct TrackSubscription {
  pub conn_id: u64,
  #[serde(flatten)]
  pub subscription: SubscriptionInfo,
}

/// A SUBSCRIBE_NAMESPACE or SUBSCRIBE_TRACKS held by a connection.
#[derive(Debug, Clone, Serialize)]
pub struct NamespaceSubscriptionInfo {
  /// The namespace prefix as `a/b`.
  pub prefix: String,
  pub conn_id: u64,
  /// `namespace` or `tracks`.
  pub kind: String,
}

/// The relay's place in a cluster.
#[derive(Debug, Clone, Serialize)]
pub struct ClusterInfo {
  /// This relay's Hop ID; `None` when it is not part of a cluster.
  pub hop_id: Option<u64>,
  /// The relays it is configured to dial.
  pub configured_peers: Vec<String>,
  pub peers: Vec<ClusterPeerInfo>,
}

/// An open link to another relay.
#[derive(Debug, Clone, Serialize)]
pub struct ClusterPeerInfo {
  pub conn_id: u64,
  pub remote: String,
  pub hop_id: Option<u64>,
  /// What the peer charges for its egress (RELAY_COST).
  pub relay_cost: u64,
  /// The namespaces this relay advertises to the peer.
  pub adverts: Vec<AdvertInfo>,
}

/// A namespace advertised to a peer, and the route it came by.
#[derive(Debug, Clone, Serialize)]
pub struct AdvertInfo {
  pub namespace: String,
  /// The connection the route came from.
  pub source_conn_id: u64,
  pub hop_path: Vec<u64>,
  pub cost: u64,
  pub generation: u64,
}
