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

//! Where the relay meets the observability layer.
//!
//! Two kinds of code live here. Snapshot functions copy the relay's state into the
//! plain types of `moqtail_probe` for the API; each holds a core lock only while it
//! copies. Helpers such as [`counters`] and [`arrival_time`] are called from the
//! delivery path to count and time objects; in a build without the `observability`
//! feature they return `None`, and the compiler removes the code that uses them.

use super::session::client::MOQTClient;
use super::session::session_context::SessionContext;
use super::subscription::Subscription;
use moqtail::model::common::location::Location;
use moqtail::model::common::pair::KeyValuePair;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::parameter::constant::SetupOptionType;
use moqtail::transport::connection::TransportConnection;
use moqtail_probe::{
  CloseInfo, ConnectionInfo, ConnectionState, PublishedTrackInfo, QuicStats, Role, SetupOptionInfo,
  SubscriptionInfo,
};
use std::time::{SystemTime, UNIX_EPOCH};
use wtransport::quinn::ConnectionError;

#[cfg(feature = "observability")]
pub(crate) use relay::RelayInspect;

/// The relay-wide traffic counters: objects and bytes received and sent, drops,
/// and cache removals. The delivery path adds to them as objects move.
///
/// `None` when the API did not install them, which happens when neither its
/// `metrics` feature nor the metrics track is on. Always `None` in a build without
/// the `observability` feature, so the counting code at each call site is removed.
#[inline]
pub(crate) fn counters() -> Option<&'static moqtail_probe::stats::RelayCounters> {
  #[cfg(feature = "observability")]
  return moqtail_probe::counters();
  #[cfg(not(feature = "observability"))]
  None
}

/// Counts a group leaving the cache, by why it left: it expired, it was evicted to
/// make room, or it was removed or replaced.
pub(crate) fn count_cache_removal(cause: moka::notification::RemovalCause) {
  use moka::notification::RemovalCause;
  if let Some(c) = counters() {
    match cause {
      RemovalCause::Expired => c.cache_expired.add(1),
      RemovalCause::Size => c.cache_evicted_for_size.add(1),
      RemovalCause::Explicit | RemovalCause::Replaced => c.cache_removed.add(1),
    }
  }
}

/// The traffic counters of the accounting zone a track's namespace falls under:
/// the longest prefix listed in `accounting_zones`. `None` when no zones are
/// configured, and always in a build without the `observability` feature.
pub(crate) fn zone_for(
  name: &FullTrackName,
) -> Option<std::sync::Arc<moqtail_probe::stats::ZoneCounters>> {
  #[cfg(feature = "observability")]
  return moqtail_probe::probe().zone(&namespace_text(&name.namespace));
  #[cfg(not(feature = "observability"))]
  {
    let _ = name;
    None
  }
}

/// Reads and replaces the relay's log filter while it runs. Set once at startup,
/// and only when the API's `actions` feature is on; otherwise the filter is fixed.
#[cfg(feature = "observability")]
type SetLogFilter = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

#[cfg(feature = "observability")]
struct LogFilter {
  get: Box<dyn Fn() -> Option<String> + Send + Sync>,
  set: SetLogFilter,
}

#[cfg(feature = "observability")]
static LOG_FILTER: std::sync::OnceLock<LogFilter> = std::sync::OnceLock::new();

/// Stores the handle that swaps the log filter, so the API can read and replace it
/// through [`moqtail_probe::Actions`].
#[cfg(feature = "observability")]
pub(crate) fn keep_log_filter<S: 'static>(
  handle: tracing_subscriber::reload::Handle<tracing_subscriber::EnvFilter, S>,
) {
  let reader = handle.clone();
  let _ = LOG_FILTER.set(LogFilter {
    get: Box::new(move || reader.with_current(|f| f.to_string()).ok()),
    set: Box::new(move |text| {
      let filter = tracing_subscriber::EnvFilter::try_new(text).map_err(|e| e.to_string())?;
      handle.reload(filter).map_err(|e| e.to_string())
    }),
  });
}

/// The moment an object arrived from its publisher. Its processing time is
/// measured from here to when it is written to each subscriber. `None` in a build
/// without the `observability` feature, so the delivery path reads no clock.
pub(crate) fn arrival_time() -> Option<std::time::Instant> {
  #[cfg(feature = "observability")]
  return Some(std::time::Instant::now());
  #[cfg(not(feature = "observability"))]
  None
}

/// How the API reads the relay ([`moqtail_probe::Inspect`]) and changes it
/// ([`moqtail_probe::Actions`]). Built
/// only with the `observability` feature.
#[cfg(feature = "observability")]
mod relay {
  use super::LOG_FILTER;
  use super::{
    connection_info, location_text, millis_since_epoch, namespace_text, subscription_info,
  };
  use crate::server::Server;
  use crate::server::session::client::MOQTClient;
  use crate::server::track::track_manager::{SubscribeKind, TrackManager};
  use crate::server::track::{Track, TrackOrigin, TrackStatus};
  use moqtail::model::control::control_message::ControlMessage;
  use moqtail::model::control::goaway::GoAway;
  use moqtail::model::error::TerminationCode;
  use moqtail_probe::{
    Actions, AdvertInfo, BoxFuture, ClusterInfo, ClusterPeerInfo, ConnectionInfo, Inspect,
    NamespaceSubscriptionInfo, RelayStatus, TrackDetail, TrackInfo, TrackPublisher,
    TrackSubscription,
  };
  use std::sync::atomic::Ordering;

  /// A handle on the server, through which the API reads and changes the relay.
  pub(crate) struct RelayInspect(pub Server);

  impl Inspect for RelayInspect {
    fn status(&self) -> RelayStatus {
      let server = &self.0;
      RelayStatus {
        version: env!("MOQTAIL_VERSION").to_string(),
        pid: std::process::id(),
        started_at_ms: millis_since_epoch(server.started_at),
        accepting: server.accepting.load(Ordering::Relaxed),
        draining: server.draining.load(Ordering::Relaxed),
      }
    }

    fn config(&self) -> serde_json::Value {
      // The tokens are marked to be skipped when serializing, so they never appear here.
      serde_json::to_value(self.0.app_config).unwrap_or_default()
    }

    fn connections(&self) -> BoxFuture<'_, Vec<ConnectionInfo>> {
      Box::pin(async move {
        // Copy the client list and release the lock before reading each client.
        let clients: Vec<_> = self
          .0
          .client_manager
          .clients
          .read()
          .await
          .values()
          .cloned()
          .collect();
        let draining = self.0.draining.load(Ordering::Relaxed);
        let mut connections = Vec::with_capacity(clients.len());
        for client in clients {
          connections.push(connection_info(&client, draining, None).await);
        }
        connections
      })
    }

    fn tracks(&self) -> BoxFuture<'_, Vec<TrackInfo>> {
      Box::pin(async move {
        let mut tracks = Vec::new();
        for track in self.0.track_manager.all_tracks().await {
          tracks.push(track_info(&track).await);
        }
        tracks
      })
    }

    fn track(&self, track_id: u64) -> BoxFuture<'_, Option<TrackDetail>> {
      Box::pin(async move {
        let tracks = self.0.track_manager.all_tracks().await;
        let track = tracks.iter().find(|t| t.relay_track_id == track_id)?;
        Some(track_detail(track).await)
      })
    }

    fn namespace_subscriptions(&self) -> BoxFuture<'_, Vec<NamespaceSubscriptionInfo>> {
      Box::pin(namespace_subscriptions(&self.0.track_manager))
    }

    fn cluster(&self) -> BoxFuture<'_, ClusterInfo> {
      Box::pin(async move {
        let clients: Vec<_> = self
          .0
          .client_manager
          .clients
          .read()
          .await
          .values()
          .filter(|c| c.peer_hop_id.is_some())
          .cloned()
          .collect();
        let mut peers = Vec::new();
        for client in clients {
          let mut adverts: Vec<AdvertInfo> = client
            .cluster_adverts
            .lock()
            .await
            .iter()
            .map(|(namespace, advert)| {
              let (source, hops, cost, generation) = advert.describe();
              AdvertInfo {
                namespace: namespace_text(namespace),
                source_conn_id: source as u64,
                hop_path: hops.to_vec(),
                cost,
                generation,
              }
            })
            .collect();
          adverts.sort_by(|a, b| a.namespace.cmp(&b.namespace));
          peers.push(ClusterPeerInfo {
            conn_id: client.connection_id as u64,
            remote: client.connection.remote_address().to_string(),
            hop_id: client.peer_hop_id,
            relay_cost: client.peer_relay_cost,
            adverts,
          });
        }
        peers.sort_by_key(|p| p.conn_id);
        ClusterInfo {
          hop_id: self.0.app_config.hop_id,
          configured_peers: self.0.app_config.peers.clone(),
          peers,
        }
      })
    }
  }

  impl RelayInspect {
    async fn client(&self, conn_id: u64) -> Option<std::sync::Arc<MOQTClient>> {
      let id = usize::try_from(conn_id).ok()?;
      self.0.client_manager.clients.read().await.get(&id).cloned()
    }

    /// Closes `client`'s connection with GOAWAY_TIMEOUT if it is still open after
    /// `timeout_ms`. GOAWAY only asks a client to leave; one that ignores it would
    /// otherwise stay for good.
    fn close_after(&self, client: std::sync::Arc<MOQTClient>, timeout_ms: u64) {
      let clients = self.0.client_manager.clients.clone();
      tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
        let still_open = clients
          .read()
          .await
          .get(&client.connection_id)
          .is_some_and(|c| std::sync::Arc::ptr_eq(c, &client));
        if still_open {
          tracing::info!(
            "Connection {} did not leave within {timeout_ms} ms of GOAWAY; closing it",
            client.connection_id
          );
          client
            .connection
            .close(TerminationCode::GoawayTimeout as u32, b"GOAWAY timeout");
        }
      });
    }
  }

  impl Actions for RelayInspect {
    fn goaway(&self, conn_id: u64, timeout_ms: u64) -> BoxFuture<'_, bool> {
      Box::pin(async move {
        let Some(client) = self.client(conn_id).await else {
          return false;
        };
        let goaway = GoAway::new(self.0.redirect_uri().await, timeout_ms);
        client
          .queue_message(ControlMessage::Goaway(Box::new(goaway)))
          .await;
        self.close_after(client, timeout_ms);
        true
      })
    }

    fn close(&self, conn_id: u64) -> BoxFuture<'_, bool> {
      Box::pin(async move {
        let Some(client) = self.client(conn_id).await else {
          return false;
        };
        // The session task notices the closed connection and cleans up as it does
        // for any other close.
        client
          .connection
          .close(TerminationCode::NoError as u32, b"closed by the operator");
        true
      })
    }

    fn drain(&self, timeout_ms: u64) -> BoxFuture<'_, usize> {
      Box::pin(async move {
        self.0.draining.store(true, Ordering::Relaxed);
        let told = self.0.broadcast_goaway(timeout_ms).await;
        let clients: Vec<_> = self
          .0
          .client_manager
          .clients
          .read()
          .await
          .values()
          .cloned()
          .collect();
        for client in clients {
          self.close_after(client, timeout_ms);
        }
        told
      })
    }

    fn redirect_uri(&self) -> BoxFuture<'_, Option<String>> {
      Box::pin(self.0.redirect_uri())
    }

    fn set_redirect_uri(&self, uri: Option<String>) -> BoxFuture<'_, ()> {
      Box::pin(async move {
        *self.0.redirect_uri.write().await = uri;
      })
    }

    fn log_filter(&self) -> Option<String> {
      LOG_FILTER.get().and_then(|f| (f.get)())
    }

    fn set_log_filter(&self, filter: &str) -> Result<(), String> {
      let log_filter = LOG_FILTER
        .get()
        .ok_or("the log filter was not made changeable at startup")?;
      (log_filter.set)(filter)
    }
  }

  async fn track_info(track: &Track) -> TrackInfo {
    let publishers = track
      .publisher_aliases
      .read()
      .await
      .iter()
      .map(|(conn_id, alias)| TrackPublisher {
        conn_id: *conn_id as u64,
        alias: *alias,
      })
      .collect();
    let status = match track.status().await {
      TrackStatus::Pending => "pending",
      TrackStatus::Confirmed { .. } => "confirmed",
      TrackStatus::Rejected { .. } => "rejected",
    };
    let origin = match track.origin() {
      TrackOrigin::Publish => "publish",
      TrackOrigin::Subscribe => "subscribe",
      TrackOrigin::CacheOnly => "cache_only",
    };
    TrackInfo {
      track_id: track.relay_track_id,
      namespace: namespace_text(&track.full_track_name.namespace),
      name: super::field_text(&track.full_track_name.name),
      origin: origin.to_string(),
      status: status.to_string(),
      publishers,
      subscribers: track.subscriber_count().await,
      forwarding: track.has_forwarding_subscriber().await,
      largest_location: track.largest_object().await.as_ref().map(location_text),
      cached_groups: track.cache.get_cache_stats().await.0,
      upstream_forward: track
        .upstream_forward
        .load(std::sync::atomic::Ordering::Relaxed),
    }
  }

  async fn track_detail(track: &Track) -> TrackDetail {
    let mut subscriptions = Vec::new();
    for subscription in track.subscription_manager.all().await {
      let subscription = subscription.read().await;
      subscriptions.push(TrackSubscription {
        conn_id: subscription.subscriber().connection_id as u64,
        subscription: subscription_info(subscription.request_id, &subscription).await,
      });
    }
    subscriptions.sort_by_key(|s| (s.conn_id, s.subscription.request_id));
    TrackDetail {
      track: track_info(track).await,
      subscriptions,
      cached_group_ids: track.cache.group_ids(),
      cached_bytes: track.cache.payload_bytes().await,
    }
  }

  async fn namespace_subscriptions(tracks: &TrackManager) -> Vec<NamespaceSubscriptionInfo> {
    let mut list: Vec<_> = tracks
      .namespace_subscriptions()
      .await
      .into_iter()
      .map(|(prefix, conn_id, kind)| NamespaceSubscriptionInfo {
        prefix: namespace_text(&prefix),
        conn_id: conn_id as u64,
        kind: match kind {
          SubscribeKind::Namespace => "namespace",
          SubscribeKind::Tracks => "tracks",
        }
        .to_string(),
      })
      .collect();
    list.sort_by(|a, b| (&a.prefix, a.conn_id).cmp(&(&b.prefix, b.conn_id)));
    list
  }
}

/// Hands a closing connection to the probe, as it was at the moment it closed, so
/// the API can list it among closed connections. Call it before the session's
/// state is cleaned up, while there is still something to copy.
pub(crate) async fn report_closed(context: &SessionContext) {
  let probe = moqtail_probe::probe();
  if !probe.keeps_closed_connections() {
    return;
  }
  if let Some(client) = context.get_client().await {
    let close = close_info(&context.connection);
    let info = connection_info(&client, false, Some(close)).await;
    probe.connection_closed(info);
  }
}

async fn connection_info(
  client: &MOQTClient,
  draining: bool,
  close: Option<CloseInfo>,
) -> ConnectionInfo {
  let announced_namespaces: Vec<String> = client
    .announced_track_namespaces
    .read()
    .await
    .iter()
    .map(namespace_text)
    .collect();
  let mut published_tracks: Vec<PublishedTrackInfo> = client
    .published_tracks
    .read()
    .await
    .iter()
    .map(|(request_id, name)| PublishedTrackInfo {
      request_id: *request_id,
      track: track_text(name),
    })
    .collect();
  published_tracks.sort_by_key(|t| t.request_id);

  let mut subscriptions = Vec::new();
  for (request_id, subscription) in client.subscriptions.all().await {
    if let Some(subscription) = subscription.upgrade() {
      subscriptions.push(subscription_info(request_id, &*subscription.read().await).await);
    }
  }
  subscriptions.sort_by_key(|s| s.request_id);

  let has_requests =
    !announced_namespaces.is_empty() || !published_tracks.is_empty() || !subscriptions.is_empty();
  let state = if close.is_some() {
    ConnectionState::Closed
  } else if draining {
    ConnectionState::Draining
  } else if has_requests {
    ConnectionState::Active
  } else {
    ConnectionState::Idle
  };

  let (implementation, setup_options) = setup_options(&client.client_setup.setup_options);
  ConnectionInfo {
    conn_id: client.connection_id as u64,
    state,
    transport: client.transport_kind.to_string(),
    remote: client.connection.remote_address().to_string(),
    role: if client.peer_hop_id.is_some() {
      Role::ClusterPeer
    } else {
      Role::Client
    },
    connected_at_ms: millis_since_epoch(client.connected_at),
    implementation,
    setup_options,
    announced_namespaces,
    published_tracks,
    subscriptions,
    quic: quic_stats(&client.connection),
    close,
  }
}

async fn subscription_info(request_id: u64, subscription: &Subscription) -> SubscriptionInfo {
  let state = subscription.subscription_state.read().await;
  SubscriptionInfo {
    request_id,
    track: track_text(&subscription.full_track_name),
    track_id: subscription.relay_track_id(),
    track_alias: subscription.track_alias,
    forward: state.forward,
    subscriber_priority: state.subscriber_priority,
    group_order: format!("{:?}", state.group_order),
    filter: format!("{:?}", state.location_filter),
    start: state.start_location.as_ref().map(location_text),
    end_group: state.end_group,
    last_sent: state.last_sent_max_location.as_ref().map(location_text),
    last_received: state
      .last_received_object_location
      .as_ref()
      .map(location_text),
    streams_opened: subscription.opened_stream_count(),
    queued_bytes: subscription.queued_bytes(),
  }
}

/// The implementation the peer named, and every option of its SETUP with a
/// readable name. Authorization tokens are never copied out.
fn setup_options(options: &[KeyValuePair]) -> (Option<String>, Vec<SetupOptionInfo>) {
  let mut implementation = None;
  let infos = options
    .iter()
    .map(|kvp| {
      let ty = kvp.get_type();
      let known = SetupOptionType::try_from(ty).ok();
      let name = match known {
        Some(SetupOptionType::Path) => "path".to_string(),
        Some(SetupOptionType::AuthorizationToken) => "authorization_token".to_string(),
        Some(SetupOptionType::MaxAuthTokenCacheSize) => "max_auth_token_cache_size".to_string(),
        Some(SetupOptionType::Authority) => "authority".to_string(),
        Some(SetupOptionType::MaxFilterRanges) => "max_filter_ranges".to_string(),
        Some(SetupOptionType::MoqtImplementation) => "implementation".to_string(),
        Some(SetupOptionType::MaxRequestUpdates) => "max_request_updates".to_string(),
        Some(SetupOptionType::SstsAlgorithms) => "ssts_algorithms".to_string(),
        None => format!("{ty:#x}"),
      };
      let value = match (known, kvp) {
        (Some(SetupOptionType::AuthorizationToken), _) => "(not shown)".to_string(),
        (_, KeyValuePair::VarInt { value, .. }) => value.to_string(),
        (_, KeyValuePair::Bytes { value, .. }) => match std::str::from_utf8(value) {
          Ok(text) => text.to_string(),
          Err(_) => value.iter().map(|b| format!("{b:02x}")).collect(),
        },
      };
      if known == Some(SetupOptionType::MoqtImplementation) {
        implementation = Some(value.clone());
      }
      SetupOptionInfo { name, value }
    })
    .collect();
  (implementation, infos)
}

fn quic_stats(connection: &TransportConnection) -> QuicStats {
  let stats = connection.quic_stats();
  QuicStats {
    rtt_ms: stats.path.rtt.as_secs_f64() * 1000.0,
    cwnd: stats.path.cwnd,
    congestion_events: stats.path.congestion_events,
    lost_packets: stats.path.lost_packets,
    lost_bytes: stats.path.lost_bytes,
    sent_bytes: stats.udp_tx.bytes,
    received_bytes: stats.udp_rx.bytes,
    sent_datagrams: stats.udp_tx.datagrams,
    received_datagrams: stats.udp_rx.datagrams,
  }
}

pub(crate) fn close_info(connection: &TransportConnection) -> CloseInfo {
  let at_ms = millis_since_epoch(SystemTime::now());
  let (by, code, reason) = match connection.close_reason() {
    Some(ConnectionError::ApplicationClosed(close)) => (
      "peer",
      Some(close.error_code.into_inner()),
      String::from_utf8_lossy(&close.reason).into_owned(),
    ),
    Some(ConnectionError::ConnectionClosed(close)) => (
      "peer",
      Some(u64::from(close.error_code)),
      String::from_utf8_lossy(&close.reason).into_owned(),
    ),
    Some(ConnectionError::LocallyClosed) => ("relay", None, "closed by the relay".to_string()),
    Some(ConnectionError::TimedOut) => ("network", None, "idle timeout".to_string()),
    Some(ConnectionError::Reset) => ("network", None, "reset".to_string()),
    Some(other) => ("relay", None, other.to_string()),
    None => ("relay", None, "unknown".to_string()),
  };
  CloseInfo {
    at_ms,
    by: by.to_string(),
    code,
    reason,
  }
}

fn track_text(name: &FullTrackName) -> String {
  format!(
    "{} : {}",
    namespace_text(&name.namespace),
    field_text(&name.name)
  )
}

/// A namespace as its fields joined by `/`, for example `live/room1`.
fn namespace_text(namespace: &Tuple) -> String {
  let fields: Vec<String> = namespace.fields.iter().map(field_text).collect();
  fields.join("/")
}

/// A namespace or name field as text, shown as written rather than escaped, so
/// `metrics.moq.arpa` reads as is. Bytes that are not valid UTF-8 show as the
/// replacement character.
fn field_text(field: &moqtail::model::common::tuple::TupleField) -> String {
  String::from_utf8_lossy(field.as_bytes()).into_owned()
}

fn location_text(location: &Location) -> String {
  format!("{}:{}", location.group, location.object)
}

fn millis_since_epoch(time: SystemTime) -> u64 {
  time
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0)
}
