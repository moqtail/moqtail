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
  collections::{BTreeMap, HashMap},
  sync::Arc,
};
use tokio::sync::RwLock;
use wtransport::Connection;

use moqtail::{
  model::data::full_track_name::FullTrackName,
  transport::data_stream_handler::{FetchRequest, SubscribeRequest},
};

use super::{client::MOQTClient, client_manager::ClientManager, config::AppConfig, track::Track};

pub struct RequestMaps {
  pub relay_fetch_requests: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
  pub client_fetch_requests: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
  pub relay_subscribe_requests: Arc<RwLock<BTreeMap<u64, SubscribeRequest>>>,
  pub client_subscribe_requests: Arc<RwLock<BTreeMap<u64, SubscribeRequest>>>,
}

pub struct SessionContext {
  pub(crate) client_manager: Arc<RwLock<ClientManager>>,
  pub(crate) tracks: Arc<RwLock<HashMap<FullTrackName, Track>>>, // the tracks the relay is subscribed to, key is the track alias
  pub(crate) track_aliases: Arc<RwLock<BTreeMap<u64, FullTrackName>>>, // the track alias and full track names
  pub(crate) relay_fetch_requests: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
  pub(crate) _client_fetch_requests: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
  pub(crate) relay_subscribe_requests: Arc<RwLock<BTreeMap<u64, SubscribeRequest>>>,
  pub(crate) client_subscribe_requests: Arc<RwLock<BTreeMap<u64, SubscribeRequest>>>,
  pub(crate) connection_id: usize,
  pub(crate) client: Arc<RwLock<Option<Arc<MOQTClient>>>>, // the client that is connected to this session
  pub(crate) connection: Connection,
  pub(crate) server_config: &'static AppConfig,
  pub(crate) is_connection_closed: Arc<RwLock<bool>>,
  pub(crate) relay_next_request_id: Arc<RwLock<u64>>,
  pub(crate) max_request_id: Arc<RwLock<u64>>,
}

impl SessionContext {
  pub fn new(
    server_config: &'static AppConfig,
    client_manager: Arc<RwLock<ClientManager>>,
    tracks: Arc<RwLock<HashMap<FullTrackName, Track>>>,
    track_aliases: Arc<RwLock<BTreeMap<u64, FullTrackName>>>,
    request_maps: RequestMaps,
    connection: Connection,
    relay_next_request_id: Arc<RwLock<u64>>,
  ) -> Self {
    Self {
      client_manager,
      tracks,
      track_aliases,
      relay_fetch_requests: request_maps.relay_fetch_requests,
      _client_fetch_requests: request_maps.client_fetch_requests,
      relay_subscribe_requests: request_maps.relay_subscribe_requests,
      client_subscribe_requests: request_maps.client_subscribe_requests,
      connection_id: connection.stable_id(),
      client: Arc::new(RwLock::new(None)), // initially no client is set
      connection,
      server_config,
      is_connection_closed: Arc::new(RwLock::new(false)),
      relay_next_request_id,
      max_request_id: Arc::new(RwLock::new(server_config.initial_max_request_id)),
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
