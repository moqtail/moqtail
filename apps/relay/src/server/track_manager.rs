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

use super::track::Track;
use crate::server::client::MOQTClient;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Clone)]
pub struct TrackManager {
  pub tracks: Arc<RwLock<HashMap<FullTrackName, Arc<RwLock<Track>>>>>,
  pub track_aliases: Arc<RwLock<BTreeMap<u64, FullTrackName>>>,
  pub namespace_subscribers: Arc<RwLock<HashMap<Tuple, Vec<Arc<MOQTClient>>>>>,
  pub announcements: Arc<RwLock<HashMap<Tuple, Arc<MOQTClient>>>>,
}

impl TrackManager {
  pub fn new() -> Self {
    TrackManager {
      tracks: Arc::new(RwLock::new(HashMap::new())),
      track_aliases: Arc::new(RwLock::new(BTreeMap::new())),
      namespace_subscribers: Arc::new(RwLock::new(HashMap::new())),
      announcements: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  pub async fn add_track(
    &self,
    track_alias: u64,
    full_track_name: FullTrackName,
    track: Track,
  ) -> Arc<RwLock<Track>> {
    let mut tracks = self.tracks.write().await;
    let mut track_aliases = self.track_aliases.write().await;

    let track = Arc::new(RwLock::new(track));
    tracks.insert(full_track_name.clone(), track.clone());
    track_aliases.insert(track_alias, full_track_name.clone());

    info!(
      "Added track with alias {}: {:?}",
      track_alias, full_track_name
    );

    track
  }

  pub async fn get_track_by_alias(&self, track_alias: u64) -> Option<Arc<RwLock<Track>>> {
    let track_aliases = self.track_aliases.read().await;
    if let Some(full_track_name) = track_aliases.get(&track_alias) {
      let tracks = self.tracks.read().await;
      return tracks.get(full_track_name).cloned();
    }
    None
  }

  pub async fn remove_track_by_alias(&self, track_alias: u64) {
    let mut track_aliases = self.track_aliases.write().await;
    if let Some(full_track_name) = track_aliases.remove(&track_alias) {
      let mut tracks = self.tracks.write().await;
      tracks.remove(&full_track_name);
      info!(
        "Removed track with alias {}: {:?}",
        track_alias, full_track_name
      );
    }
  }

  pub async fn remove_track(&self, full_track_name: &FullTrackName) {
    let mut tracks = self.tracks.write().await;
    if tracks.remove(full_track_name).is_some() {
      info!("Removed track by name: {:?}", full_track_name);
    }
  }

  pub async fn get_track(&self, full_track_name: &FullTrackName) -> Option<Arc<RwLock<Track>>> {
    let tracks = self.tracks.read().await;
    tracks.get(full_track_name).cloned()
  }

  pub async fn has_track(&self, full_track_name: &FullTrackName) -> bool {
    let tracks = self.tracks.read().await;
    tracks.contains_key(full_track_name)
  }

  pub async fn has_track_alias(&self, track_alias: &u64) -> bool {
    let track_aliases = self.track_aliases.read().await;
    track_aliases.contains_key(track_alias)
  }

  /// Atomically gets an existing track or creates a new one.
  /// Returns the track Arc and a boolean indicating whether this call created the track.
  pub async fn get_or_create_track(
    &self,
    full_track_name: &FullTrackName,
    track_factory: impl FnOnce() -> Track,
  ) -> (Arc<RwLock<Track>>, bool) {
    // Fast path: read lock
    {
      let tracks = self.tracks.read().await;
      if let Some(track) = tracks.get(full_track_name) {
        return (track.clone(), false);
      }
    }
    // Slow path: write lock with double-check
    {
      let mut tracks = self.tracks.write().await;
      if let Some(track) = tracks.get(full_track_name) {
        return (track.clone(), false);
      }
      let track = track_factory();
      let track_arc = Arc::new(RwLock::new(track));
      tracks.insert(full_track_name.clone(), track_arc.clone());
      // Do NOT insert into track_aliases -- the publisher alias is
      // unknown until SubscribeOk arrives. Use add_track_alias() later.
      (track_arc, true)
    }
  }

  /// Register a track alias mapping. Called when the publisher's SubscribeOk
  /// reveals the actual track_alias.
  pub async fn add_track_alias(&self, track_alias: u64, full_track_name: FullTrackName) {
    let mut track_aliases = self.track_aliases.write().await;
    track_aliases.insert(track_alias, full_track_name.clone());
    info!(
      "Registered track alias {} -> {:?}",
      track_alias, full_track_name
    );
  }

  pub async fn add_namespace_subscriber(&self, prefix: Tuple, client: Arc<MOQTClient>) {
    let mut subs = self.namespace_subscribers.write().await;

    // Get or create the list for this prefix
    let clients = subs.entry(prefix.clone()).or_insert_with(Vec::new);

    // Avoid duplicates
    if !clients
      .iter()
      .any(|c| c.connection_id == client.connection_id)
    {
      clients.push(client);
    }
    info!("Added namespace subscriber for prefix {:?}", prefix);
  }

  pub async fn get_namespace_subscribers(&self, target_namespace: &Tuple) -> Vec<Arc<MOQTClient>> {
    let subs = self.namespace_subscribers.read().await;
    let mut interested_clients = Vec::new();

    // Check every prefix. If target starts with prefix, they are interested.
    // Example: Target "meet.room1", Prefix "meet" -> Match.
    for (prefix, clients) in subs.iter() {
      if target_namespace.starts_with(prefix) {
        for client in clients {
          interested_clients.push(client.clone());
        }
      }
    }
    interested_clients
  }

  pub async fn add_announcement(&self, namespace: Tuple, publisher: Arc<MOQTClient>) {
    let mut announcements = self.announcements.write().await;
    announcements.insert(namespace.clone(), publisher);
    info!("Stored announcement for namespace: {:?}", namespace);
  }

  pub async fn remove_announcements_by_connection(&self, connection_id: usize) {
    let mut announcements = self.announcements.write().await;
    announcements.retain(|ns, client| {
      if client.connection_id == connection_id {
        info!(
          "Removed announcement for namespace {:?} (publisher {} disconnected)",
          ns, connection_id
        );
        false
      } else {
        true
      }
    });
  }

  pub async fn remove_namespace_subscriber(&self, connection_id: usize) {
    let mut subs = self.namespace_subscribers.write().await;
    for (prefix, clients) in subs.iter_mut() {
      let before = clients.len();
      clients.retain(|c| c.connection_id != connection_id);
      if clients.len() < before {
        info!(
          "Removed namespace subscriber {} from prefix {:?}",
          connection_id, prefix
        );
      }
    }
    subs.retain(|_, clients| !clients.is_empty());
  }

  pub async fn get_announcements_by_prefix(&self, prefix: &Tuple) -> Vec<Tuple> {
    let announcements = self.announcements.read().await;
    let mut matches = Vec::new();

    for (ns, _) in announcements.iter() {
      if ns.starts_with(prefix) {
        matches.push(ns.clone());
      }
    }

    matches
  }
}
