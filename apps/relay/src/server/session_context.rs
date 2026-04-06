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
  collections::BTreeMap,
  sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64},
  },
};
use tokio::sync::RwLock;
use wtransport::Connection;

use moqtail::transport::data_stream_handler::{FetchRequest, SubscribeRequest};

use super::{
  client::MOQTClient, client_manager::ClientManager, config::AppConfig, track_manager::TrackManager,
};

#[allow(dead_code)]
//These changes are required for message unification and will be useful in other branches, hence we allow the dead code here
#[derive(Debug, Clone)]
pub enum PendingRequest {
  Fetch(FetchRequest),
  Subscribe(SubscribeRequest),
  TrackStatus(SubscribeRequest),
  PublishNamespace {
    client_connection_id: usize,
    original_request_id: u64,
    message: moqtail::model::control::publish_namespace::PublishNamespace,
  },
  SubscribeNamespace {
    client_connection_id: usize,
    original_request_id: u64,
    message: moqtail::model::control::subscribe_namespace::SubscribeNamespace,
  },
  Publish {
    publisher_connection_id: usize,
    original_request_id: u64,
    message: moqtail::model::control::publish::Publish,
  },
  RequestUpdate {
    client_connection_id: usize,
    original_request_id: u64,
    message: moqtail::model::control::request_update::RequestUpdate,
  },
}

pub struct RequestMaps {
  pub relay_pending_requests: Arc<RwLock<BTreeMap<u64, PendingRequest>>>,
}

pub struct SessionContext {
  pub(crate) client_manager: Arc<RwLock<ClientManager>>,
  pub(crate) track_manager: TrackManager,
  pub(crate) relay_pending_requests: Arc<RwLock<BTreeMap<u64, PendingRequest>>>,
  pub(crate) connection_id: usize,
  pub(crate) client: Arc<RwLock<Option<Arc<MOQTClient>>>>, // the client that is connected to this session
  pub(crate) connection: Connection,
  pub(crate) server_config: &'static AppConfig,
  pub(crate) is_connection_closed: Arc<AtomicBool>,
  pub(crate) relay_next_request_id: Arc<AtomicU64>,
  pub(crate) max_request_id: Arc<AtomicU64>,
}

impl SessionContext {
  pub fn new(
    server_config: &'static AppConfig,
    client_manager: Arc<RwLock<ClientManager>>,
    track_manager: TrackManager,
    request_maps: RequestMaps,
    connection: Connection,
    relay_next_request_id: Arc<AtomicU64>,
  ) -> Self {
    Self {
      client_manager,
      track_manager,
      relay_pending_requests: request_maps.relay_pending_requests,
      connection_id: connection.stable_id(),
      client: Arc::new(RwLock::new(None)), // initially no client is set
      connection,
      server_config,
      is_connection_closed: Arc::new(AtomicBool::new(false)),
      relay_next_request_id,
      max_request_id: Arc::new(AtomicU64::new(server_config.initial_max_request_id)),
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
