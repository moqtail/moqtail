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

//! The interface between the relay core and its observability layer.
//!
//! The core implements [`Inspect`] so the observability layer can read its state as
//! plain copies, without touching the core's locks. Everything that serves or
//! exports that state lives in the observability crate, which the core never
//! imports.

use serde::Serialize;
use std::future::Future;
use std::pin::Pin;

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
}

/// What the relay core reports as things happen. Every method has an empty default,
/// so the core can always call them, and [`NoProbe`] is the probe when nothing
/// observes the relay.
pub trait Probe: Send + Sync {
  /// True when [`Probe::connection_closed`] keeps what it is given. The core builds a
  /// closing connection's summary only then.
  fn keeps_closed_connections(&self) -> bool {
    false
  }

  /// A connection closed. `connection` describes it as it was at that moment, with
  /// its close reason set.
  fn connection_closed(&self, _connection: ConnectionInfo) {}
}

/// The probe that ignores everything.
pub struct NoProbe;

impl Probe for NoProbe {}

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
