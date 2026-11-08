// Copyright 2025 The MOQtail Authors
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

use std::{
  hash::{BuildHasher, Hasher, RandomState},
  sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
  },
};
use tokio::sync::RwLock;

use moqtail::model::common::location::Location;
use moqtail::model::data::fetch_object::{EndOfRangeKind, FetchObjectPayload};

use moqtail::transport::connection::{TransportConnection, TransportKind};

use super::{
  client::MOQTClient, client_manager::ClientManager, config::AppConfig, requests::UpstreamRequests,
  track_manager::TrackManager,
};

/// This is used when the relay sends a FETCH request to the upstream.
/// It gets an event of this type from the upstream, and calls send() on
/// the sender `UpstreamRequests` holds for it.
/// The receiver awaits entries in a loop and sends them downstream.
#[allow(dead_code)]
pub(crate) enum UpstreamFetchEvent {
  /// The publisher's FETCH_OK, carrying the range it will actually deliver. A relay
  /// cannot answer its own subscriber until it has this.
  Accepted {
    end_location: Location,
  },
  Object(FetchObjectPayload),
  /// Every Location after the previous Object, if any, up to and including
  /// `location`, is of `kind`.
  EndOfRange {
    kind: EndOfRangeKind,
    location: Location,
  },
  MalformedTrack,
  StreamClosed,
  Error(String),
}

pub struct SessionContext {
  pub(crate) client_manager: ClientManager,
  pub(crate) track_manager: TrackManager,
  pub(crate) upstream_requests: UpstreamRequests,
  pub(crate) connection_id: usize,
  pub(crate) transport_kind: TransportKind,
  pub(crate) client: Arc<RwLock<Option<Arc<MOQTClient>>>>, // the client that is connected to this session
  pub(crate) connection: TransportConnection,
  pub(crate) server_config: &'static AppConfig,
  pub(crate) is_connection_closed: Arc<AtomicBool>,
  pub(crate) relay_next_request_id: Arc<AtomicU64>,
  pub(crate) active_request_streams: Arc<AtomicU64>,
  /// Set once the relay is draining (GOAWAY sent); new requests are rejected.
  pub(crate) draining: Arc<AtomicBool>,
}

static NEXT_CONNECTION_ID: LazyLock<AtomicUsize> = LazyLock::new(|| {
  let seed = RandomState::new().build_hasher().finish() as u32;
  AtomicUsize::new(seed as usize)
});

fn next_connection_id() -> usize {
  NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed)
}

impl SessionContext {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    server_config: &'static AppConfig,
    client_manager: ClientManager,
    track_manager: TrackManager,
    upstream_requests: UpstreamRequests,
    connection: TransportConnection,
    relay_next_request_id: Arc<AtomicU64>,
    active_request_streams: Arc<AtomicU64>,
    draining: Arc<AtomicBool>,
  ) -> Self {
    Self {
      client_manager,
      track_manager,
      upstream_requests,
      connection_id: next_connection_id(),
      transport_kind: connection.kind(),
      client: Arc::new(RwLock::new(None)), // initially no client is set
      connection,
      server_config,
      is_connection_closed: Arc::new(AtomicBool::new(false)),
      relay_next_request_id,
      active_request_streams,
      draining,
    }
  }

  pub async fn set_client(&self, client: Arc<MOQTClient>) {
    let mut guard = self.client.write().await;
    *guard = Some(client);
  }

  pub async fn get_client(&self) -> Option<Arc<MOQTClient>> {
    self.client.read().await.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::next_connection_id;

  // The only test that draws ids: another one running in parallel would take ids
  // in between and throw off the difference.
  #[test]
  fn connection_ids_incrementing() {
    let first = next_connection_id();
    next_connection_id();
    next_connection_id();
    let last = next_connection_id();
    assert_eq!(last.wrapping_sub(first), 3);
  }
}
