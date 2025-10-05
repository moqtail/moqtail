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

use super::client::MOQTClient;
use moqtail::model::{common::tuple::Tuple, data::full_track_name::FullTrackName};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::{debug, info};

pub(crate) struct ClientManager {
  pub clients: Arc<RwLock<BTreeMap<usize, Arc<MOQTClient>>>>,
}

impl ClientManager {
  pub(crate) fn new() -> Self {
    ClientManager {
      clients: Arc::new(RwLock::new(BTreeMap::new())),
    }
  }

  pub(crate) async fn add(&mut self, client: Arc<MOQTClient>) {
    let connection_id = client.connection_id;
    let mut clients = self.clients.write().await;
    clients.insert(connection_id, client);
    info!("Added client connection_id: {}", connection_id);
  }

  pub(crate) async fn remove(&mut self, connection_id: usize) {
    let mut clients = self.clients.write().await;
    clients.remove(&connection_id);
  }

  pub(crate) async fn get(&self, connection_id: usize) -> Option<Arc<MOQTClient>> {
    let clients = self.clients.read().await;
    clients.get(&connection_id).cloned()
  }

  // returns the first publisher that matches the full_track_name
  // TODO: we need to handle the case where multiple publishers are publishing the same track
  pub(crate) async fn get_publisher_by_full_track_name(
    &self,
    full_track_name: &FullTrackName,
  ) -> Option<Arc<MOQTClient>> {
    let clients = self.clients.read().await;

    for client_ref in clients.iter() {
      let client = client_ref.1;
      if client
        .published_tracks
        .read()
        .await
        .contains(full_track_name)
      {
        return Some(client.clone());
      }
    }
    None
  }

  // TODO: same namespace can be used by different publishers
  // In our implementation, we expect a unique namespace is announced
  // by every publisher such as /moqtail/my_room/user_1
  // This can be solved by the Publish message.
  pub(crate) async fn get_publisher_by_announced_track_namespace(
    &self,
    track_namespace: &Tuple,
  ) -> Option<Arc<MOQTClient>> {
    let clients = self.clients.read().await;
    for client_ref in clients.iter() {
      debug!("checking client: {:?}", client_ref.0);
      let client = client_ref.1;

      debug!(
        "client announced track namespaces: {:?} track namespace: {:?}",
        client.announced_track_namespaces, track_namespace
      );
      let announced_track_namespaces = client.announced_track_namespaces.read().await;

      for announced_track_namespace in announced_track_namespaces.iter() {
        // Check if track_namespace is equal to or a child of announced_track_namespace
        if track_namespace.starts_with(announced_track_namespace) {
          return Some(client_ref.1.clone());
        }
      }
    }
    None
  }
}
