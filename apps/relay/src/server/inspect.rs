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

//! Copies of the relay's state for the observability layer, in the plain types of
//! `moqtail_probe`. Each function holds a core lock only while it copies from it.

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

/// The `Inspect` implementation, built only with the observability layer.
#[cfg(feature = "observability")]
mod relay {
  use super::{connection_info, millis_since_epoch};
  use crate::server::Server;
  use moqtail_probe::{BoxFuture, ConnectionInfo, Inspect, RelayStatus};
  use std::sync::atomic::Ordering;

  /// The relay as the observability layer sees it.
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
  }
}

/// Reports a closing session's connection to the probe, as it was when it closed.
/// Called before the session's state is cleaned up.
pub(crate) async fn report_closed(context: &SessionContext) {
  if !context.probe.keeps_closed_connections() {
    return;
  }
  if let Some(client) = context.get_client().await {
    let close = close_info(&context.connection);
    let info = connection_info(&client, false, Some(close)).await;
    context.probe.connection_closed(info);
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

fn close_info(connection: &TransportConnection) -> CloseInfo {
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
  format!("{} : {}", namespace_text(&name.namespace), name.name)
}

/// A namespace as its fields joined by `/`, for example `live/room1`.
fn namespace_text(namespace: &Tuple) -> String {
  let fields: Vec<String> = namespace.fields.iter().map(|f| f.to_string()).collect();
  fields.join("/")
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
