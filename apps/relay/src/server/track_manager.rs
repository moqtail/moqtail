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
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, info};

#[derive(Clone)]
pub struct TrackManager {
  pub tracks: Arc<RwLock<HashMap<FullTrackName, Arc<RwLock<Track>>>>>,
  /// Maps (publisher_connection_id, publisher_track_alias) -> FullTrackName.
  /// Connection-scoped to avoid alias collisions across different publishers.
  pub track_aliases: Arc<RwLock<HashMap<(usize, u64), FullTrackName>>>,
  pub namespace_subscribers: Arc<RwLock<HashMap<Tuple, Vec<Arc<MOQTClient>>>>>,
  pub announcements: Arc<RwLock<HashMap<Tuple, Arc<MOQTClient>>>>,
  /// Counter for generating stable relay_track_id values.
  next_relay_track_id: Arc<AtomicU64>,
}

impl TrackManager {
  pub fn new() -> Self {
    TrackManager {
      tracks: Arc::new(RwLock::new(HashMap::new())),
      track_aliases: Arc::new(RwLock::new(HashMap::new())),
      namespace_subscribers: Arc::new(RwLock::new(HashMap::new())),
      announcements: Arc::new(RwLock::new(HashMap::new())),
      next_relay_track_id: Arc::new(AtomicU64::new(0)),
    }
  }

  /// Generate the next unique relay_track_id. Called once per new track.
  pub fn generate_relay_track_id(&self) -> u64 {
    self.next_relay_track_id.fetch_add(1, Ordering::Relaxed)
  }

  pub async fn add_track(
    &self,
    connection_id: usize,
    track_alias: u64,
    full_track_name: FullTrackName,
    track: Track,
  ) -> Arc<RwLock<Track>> {
    let mut tracks = self.tracks.write().await;
    let mut track_aliases = self.track_aliases.write().await;

    let relay_track_id = track.relay_track_id;
    let track = Arc::new(RwLock::new(track));
    tracks.insert(full_track_name.clone(), track.clone());
    track_aliases.insert((connection_id, track_alias), full_track_name.clone());

    info!(
      "Added track relay_track_id={} publisher_alias={}@{}: {:?}",
      relay_track_id, track_alias, connection_id, full_track_name
    );

    track
  }

  pub async fn get_track_by_alias(
    &self,
    connection_id: usize,
    track_alias: u64,
  ) -> Option<Arc<RwLock<Track>>> {
    let track_aliases = self.track_aliases.read().await;
    if let Some(full_track_name) = track_aliases.get(&(connection_id, track_alias)) {
      let tracks = self.tracks.read().await;
      return tracks.get(full_track_name).cloned();
    }
    None
  }

  #[allow(dead_code)]
  pub async fn remove_track_by_alias(&self, connection_id: usize, track_alias: u64) {
    let mut track_aliases = self.track_aliases.write().await;
    if let Some(full_track_name) = track_aliases.remove(&(connection_id, track_alias)) {
      let mut tracks = self.tracks.write().await;
      tracks.remove(&full_track_name);
      info!(
        "Removed track with alias {}@{}: {:?}",
        track_alias, connection_id, full_track_name
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

  pub async fn has_track_alias(&self, connection_id: usize, track_alias: &u64) -> bool {
    let track_aliases = self.track_aliases.read().await;
    track_aliases.contains_key(&(connection_id, *track_alias))
  }

  /// Remove a single publisher alias without removing the full track entry.
  /// Used when a publisher sends PublishDone or when only one of several publishers disconnects.
  pub async fn remove_publisher_alias(&self, connection_id: usize, track_alias: u64) {
    let mut track_aliases = self.track_aliases.write().await;
    if track_aliases
      .remove(&(connection_id, track_alias))
      .is_some()
    {
      info!(
        "Removed publisher alias {}@{} from track_aliases",
        track_alias, connection_id
      );
    }
  }

  /// Atomically gets an existing track or creates a new one.
  /// The factory receives the generated relay_track_id and must use it when constructing Track.
  /// Returns the track Arc and a boolean indicating whether this call created the track.
  pub async fn get_or_create_track(
    &self,
    full_track_name: &FullTrackName,
    track_factory: impl FnOnce(u64) -> Track,
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
      let relay_track_id = self.generate_relay_track_id();
      let track = track_factory(relay_track_id);
      let track_arc = Arc::new(RwLock::new(track));
      tracks.insert(full_track_name.clone(), track_arc.clone());
      // Do NOT insert into track_aliases -- the publisher alias is
      // unknown until SubscribeOk arrives. Use add_track_alias() later.
      (track_arc, true)
    }
  }

  /// Register a track alias mapping. Called when the publisher's SubscribeOk
  /// reveals the actual track_alias, or when a publisher registers via Publish.
  pub async fn add_track_alias(
    &self,
    connection_id: usize,
    track_alias: u64,
    full_track_name: FullTrackName,
  ) {
    let mut track_aliases = self.track_aliases.write().await;
    track_aliases.insert((connection_id, track_alias), full_track_name.clone());
    info!(
      "Registered track alias {}@{} -> {:?}",
      track_alias, connection_id, full_track_name
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
    debug!(
      "get_namespace_subscribers | target_namespace: {:?}",
      target_namespace
    );
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

  /// Returns the first existing namespace subscription prefix for `connection_id`
  /// that overlaps with `new_prefix` (equal, or one is a prefix of the other).
  /// Returns `None` if no overlap is found.
  pub async fn find_overlapping_namespace_subscription(
    &self,
    connection_id: usize,
    new_prefix: &Tuple,
  ) -> Option<Tuple> {
    let subs = self.namespace_subscribers.read().await;
    for (existing_prefix, clients) in subs.iter() {
      if clients.iter().any(|c| c.connection_id == connection_id)
        && namespace_prefixes_overlap(new_prefix, existing_prefix)
      {
        return Some(existing_prefix.clone());
      }
    }
    None
  }
}

/// Returns `true` when `a` and `b` overlap — i.e. they are equal, or one is a
/// prefix of the other. Both directions are checked so the caller doesn't need
/// to worry about argument ordering.
pub(crate) fn namespace_prefixes_overlap(a: &Tuple, b: &Tuple) -> bool {
  a == b || a.starts_with(b) || b.starts_with(a)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn t(path: &str) -> Tuple {
    Tuple::from_utf8_path(path)
  }

  #[test]
  fn identical_prefixes_overlap() {
    assert!(namespace_prefixes_overlap(
      &t("meet/room1"),
      &t("meet/room1")
    ));
  }

  #[test]
  fn new_is_extension_of_existing() {
    // "meet/room1" starts with "meet" -> overlap
    assert!(namespace_prefixes_overlap(&t("meet/room1"), &t("meet")));
  }

  #[test]
  fn existing_is_extension_of_new() {
    // "meet" starts with "meet" but existing is longer: "meet/room1"
    assert!(namespace_prefixes_overlap(&t("meet"), &t("meet/room1")));
  }

  #[test]
  fn disjoint_prefixes_do_not_overlap() {
    assert!(!namespace_prefixes_overlap(&t("meet"), &t("live")));
  }

  #[test]
  fn partial_component_match_does_not_overlap() {
    // "meetup" should NOT be considered a prefix of "meet" — tuple components
    // are compared element-wise, not as substring matches.
    assert!(!namespace_prefixes_overlap(&t("meetup"), &t("meet")));
  }

  #[test]
  fn empty_prefix_overlaps_everything() {
    // An empty tuple is a prefix of any tuple, so it overlaps with all.
    assert!(namespace_prefixes_overlap(&t(""), &t("meet/room1")));
    assert!(namespace_prefixes_overlap(&t("meet/room1"), &t("")));
  }
}
