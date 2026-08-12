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
use moqtail::model::data::full_track_name::FullTrackName;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::{debug, info};

#[derive(Clone)]
pub(crate) struct ClientManager {
  pub clients: Arc<RwLock<BTreeMap<usize, Arc<MOQTClient>>>>,
}

impl ClientManager {
  pub(crate) fn new() -> Self {
    ClientManager {
      clients: Arc::new(RwLock::new(BTreeMap::new())),
    }
  }

  pub(crate) async fn add(&self, client: Arc<MOQTClient>) {
    let connection_id = client.connection_id;
    let transport_kind = client.transport_kind;
    let mut clients = self.clients.write().await;
    clients.insert(connection_id, client);
    info!(
      "Added client connection_id: {} transport: {}",
      connection_id, transport_kind
    );
  }

  pub(crate) async fn remove(&self, connection_id: usize) -> Option<Arc<MOQTClient>> {
    let mut clients = self.clients.write().await;
    clients.remove(&connection_id)
  }

  pub(crate) async fn get(&self, connection_id: usize) -> Option<Arc<MOQTClient>> {
    let clients = self.clients.read().await;
    clients.get(&connection_id).cloned()
  }

  /// Every publisher a request for this Track must reach: those already publishing the
  /// exact Track, and those that announced a namespace the Track falls under. Both are
  /// matches, so this is their union rather than a preference between them, and a
  /// publisher matching in both ways is returned once.
  pub(crate) async fn get_publishers_for_track(
    &self,
    full_track_name: &FullTrackName,
  ) -> Vec<Arc<MOQTClient>> {
    let clients = self.clients.read().await;
    let mut matched = Vec::new();

    for (connection_id, client) in clients.iter() {
      debug!("checking client: {:?}", connection_id);

      let publishes_track = client
        .published_tracks
        .read()
        .await
        .values()
        .any(|n| n == full_track_name);

      let announced_namespace = !publishes_track && {
        let announced = client.announced_track_namespaces.read().await;
        debug!(
          "client announced track namespaces: {:?} track namespace: {:?}",
          announced, full_track_name.namespace
        );
        // Equal to, or a prefix of, the Track's namespace.
        announced
          .iter()
          .any(|ns| full_track_name.namespace.starts_with(ns))
      };

      if publishes_track || announced_namespace {
        matched.push(client.clone());
      }
    }
    matched
  }

  /// Diagnostic snapshot of every client's announced namespaces and published
  /// track names. Used to log what the relay had registered at the moment a
  /// SUBSCRIBE lookup missed, so the sent name/namespace can be compared against
  /// what actually exists.
  pub(crate) async fn dump_registrations(&self) -> String {
    let clients = self.clients.read().await;
    let mut out = String::new();
    for (connection_id, client) in clients.iter() {
      let announced = client.announced_track_namespaces.read().await;
      let published = client.published_tracks.read().await;
      out.push_str(&format!(
        "\n  client {connection_id}: announced_namespaces={:?} published_tracks={:?}",
        announced.iter().collect::<Vec<_>>(),
        published.values().collect::<Vec<_>>(),
      ));
    }
    if out.is_empty() {
      out.push_str(" <no clients>");
    }
    out
  }
}
