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

use super::subscription::Subscription;
use super::track::{Cleanup, Track, TrackOrigin};
use crate::server::client::MOQTClient;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish::Publish;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::parameter::message_parameter::{
  MessageParameter, apply_message_parameter_update,
};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;
use tracing::{info, trace, warn};

pub type NamespacePrefix = Tuple;

/// The two prefix-subscription request types. They share a wire shape but have
/// independent overlap spaces: a Namespace and a Tracks subscription may share a
/// prefix, but two of the same kind may not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeKind {
  /// SUBSCRIBE_NAMESPACE (0x50): discovery — the relay sends NAMESPACE /
  /// NAMESPACE_DONE for matching namespaces.
  Namespace,
  /// SUBSCRIBE_TRACKS (0x51): the relay sends a PUBLISH for every matching track.
  Tracks,
}

/// A prefix subscription: which client, which kind, its parameters, and the
/// channel that forwards messages onto its request stream.
pub type NamespaceSubscriber = (
  Arc<MOQTClient>,
  SubscribeKind,
  Vec<MessageParameter>,
  UnboundedSender<ControlMessage>,
);
pub type AnnouncementSubscriber = (Arc<MOQTClient>, PublishNamespace);

/// The relay's tracks and what indexes them.
///
/// Lock order: take `tracks`, then a track's inner locks (such as
/// `publisher_aliases`), then `track_aliases`, then `publishes`,
/// skipping those you do not need. Never take a lock that you, or your caller,
/// already hold, even for reading: tokio's `RwLock` queues new readers behind a
/// waiting writer, so a second read deadlocks once a writer queues in between.
#[derive(Clone)]
pub struct TrackManager {
  pub tracks: Arc<RwLock<HashMap<FullTrackName, Arc<Track>>>>,
  /// Maps (publisher_connection_id, publisher_track_alias) -> FullTrackName.
  /// Connection-scoped to avoid alias collisions across different publishers.
  pub track_aliases: Arc<RwLock<HashMap<(usize, u64), FullTrackName>>>,
  pub namespace_subscribers: Arc<RwLock<HashMap<NamespacePrefix, Vec<NamespaceSubscriber>>>>,
  pub announcements: Arc<RwLock<HashMap<Tuple, AnnouncementSubscriber>>>,
  pub publishes: Arc<RwLock<HashMap<FullTrackName, HashMap<usize, Publish>>>>,
  /// Counter for generating stable relay_track_id values.
  next_relay_track_id: Arc<AtomicU64>,
  /// Fires whenever a track alias is registered, so a data stream that arrived ahead of
  /// the control message establishing its alias can wait instead of polling.
  alias_registered: Arc<Notify>,
}

impl TrackManager {
  pub fn new() -> Self {
    TrackManager {
      tracks: Arc::new(RwLock::new(HashMap::new())),
      track_aliases: Arc::new(RwLock::new(HashMap::new())),
      alias_registered: Arc::new(Notify::new()),
      namespace_subscribers: Arc::new(RwLock::new(HashMap::new())),
      announcements: Arc::new(RwLock::new(HashMap::new())),
      publishes: Arc::new(RwLock::new(HashMap::new())),
      next_relay_track_id: Arc::new(AtomicU64::new(0)),
    }
  }

  /// Generate the next unique relay_track_id. Called once per new track.
  pub fn generate_relay_track_id(&self) -> u64 {
    self.next_relay_track_id.fetch_add(1, Ordering::Relaxed)
  }

  /// Updates the stored Publish message with new parameters from a RequestUpdate.
  /// This ensures that any late-joining subscribers get the correct, updated parameters.
  pub async fn update_publish_message_parameters(
    &self,
    full_track_name: &FullTrackName,
    connection_id: usize,
    new_parameters: &[moqtail::model::parameter::message_parameter::MessageParameter],
  ) {
    let mut publishes = self.publishes.write().await;
    if let Some(map) = publishes.get_mut(full_track_name)
      && let Some(publish_msg) = map.get_mut(&connection_id)
    {
      info!(
        "Updating stored Publish message parameters for {:?}",
        full_track_name
      );
      apply_message_parameter_update(&mut publish_msg.parameters, new_parameters.to_vec());
    }
  }

  pub async fn get_track_by_alias(
    &self,
    connection_id: usize,
    track_alias: u64,
  ) -> Option<Arc<Track>> {
    // Tracks first, as the lock order says. Holding both, the lookup sees a
    // concurrent remove_track, which clears the track's aliases while it holds
    // the tracks map, either completely or not at all.
    let tracks = self.tracks.read().await;
    let track_aliases = self.track_aliases.read().await;
    let full_track_name = track_aliases.get(&(connection_id, track_alias))?;
    tracks.get(full_track_name).cloned()
  }

  pub async fn remove_track(&self, full_track_name: &FullTrackName) {
    let mut tracks = self.tracks.write().await;
    if let Some(track_guard) = tracks.remove(full_track_name) {
      info!(
        "remove_track | removed track by name: {:?}",
        full_track_name
      );
      let track = &track_guard;
      for (connection_id, track_alias) in track.publisher_aliases.read().await.iter() {
        // remove the alias mapping
        let mut track_aliases = self.track_aliases.write().await;
        if track_aliases
          .remove(&(*connection_id, *track_alias))
          .is_none()
        {
          warn!(
            "remove_track | track alias could not removed for connection id: {} and track_alias: {}",
            connection_id, track_alias
          );
        }
      }
    } else {
      warn!(
        "remove_track | track not found to remove: {:?}",
        full_track_name
      );
    }

    let mut publishes = self.publishes.write().await;
    if publishes.remove(full_track_name).is_none() {
      warn!(
        "remove_track | publish could not removed for {:?}",
        full_track_name
      );
    }
  }

  /// Carries out the clean-up a track left once it let go of its upstream.
  pub async fn clean_up(&self, full_track_name: &FullTrackName, cleanup: Cleanup) {
    match cleanup {
      Cleanup::Nothing => {}
      Cleanup::RemoveAliases(aliases) => {
        for (connection_id, track_alias) in aliases {
          self
            .remove_publisher_alias(connection_id, track_alias)
            .await;
        }
        info!(
          "Kept track {:?} for what it has cached; nothing feeds it upstream",
          full_track_name
        );
      }
      Cleanup::RemoveTrack => self.remove_track(full_track_name).await,
    }
  }

  /// Removes the cache-only tracks whose cache holds nothing, such as one emptied by
  /// expiry or one whose FETCH brought nothing back, and that nothing else holds: no
  /// FETCH delivering from it, no SUBSCRIBE or PUBLISH taking it over. Returns how
  /// many.
  pub async fn sweep_empty_cache_only_tracks(&self) -> usize {
    let mut tracks = self.tracks.write().await;
    let mut unused = Vec::new();
    for (name, track) in tracks.iter() {
      // Held only here, and nobody can take it while the map is locked.
      if Arc::strong_count(track) != 1 {
        continue;
      }
      if track.origin() == TrackOrigin::CacheOnly && track.cache.is_empty().await {
        unused.push(name.clone());
      }
    }
    // A cache-only track has no publisher aliases: they are cleared when it becomes
    // one, and a takeover changes its origin.
    for name in &unused {
      tracks.remove(name);
    }
    drop(tracks);
    let mut publishes = self.publishes.write().await;
    for name in &unused {
      publishes.remove(name);
    }
    unused.len()
  }

  /// The tracks, copied out so callers can visit them without holding the map,
  /// which would stall track creation and removal.
  async fn snapshot(&self) -> Vec<(FullTrackName, Arc<Track>)> {
    let tracks = self.tracks.read().await;
    tracks
      .iter()
      .map(|(name, track)| (name.clone(), track.clone()))
      .collect()
  }

  /// Takes a closed connection off every track it published. Returns the tracks no
  /// publisher serves any more.
  pub async fn remove_publisher_everywhere(&self, connection_id: usize) -> Vec<FullTrackName> {
    let mut unserved = Vec::new();
    for (name, track) in self.snapshot().await {
      if let Some(removal) = track.remove_publisher(connection_id).await {
        self
          .remove_publisher_alias(connection_id, removal.alias)
          .await;
        if !removal.still_served {
          unserved.push(name);
        }
      }
    }
    unserved
  }

  /// Ends every subscription a closed connection held.
  pub async fn remove_subscriber_everywhere(&self, connection_id: usize) {
    for (_, track) in self.snapshot().await {
      track.remove_subscriptions(connection_id).await;
    }
  }

  /// Lets go of a track nothing upstream serves any more (see `Track::retire`).
  pub async fn retire_track(&self, full_track_name: &FullTrackName) {
    let Some(track) = self.get_track(full_track_name).await else {
      return;
    };
    let cleanup = track.retire().await;
    self.clean_up(full_track_name, cleanup).await;
  }

  pub async fn get_track(&self, full_track_name: &FullTrackName) -> Option<Arc<Track>> {
    let tracks = self.tracks.read().await;
    tracks.get(full_track_name).cloned()
  }

  /// Find the subscription a connection holds under the given request id, across
  /// all tracks and however it was created. Resolves the subscription a
  /// SWITCH_FROM names.
  pub async fn find_subscription_by_request_id(
    &self,
    connection_id: usize,
    request_id: u64,
  ) -> Option<(Arc<Track>, Arc<RwLock<Subscription>>)> {
    let track_arcs: Vec<Arc<Track>> = {
      let tracks = self.tracks.read().await;
      tracks.values().cloned().collect()
    };
    for track_arc in track_arcs {
      let sub_opt = {
        let track = &track_arc;
        track.get_subscription(&(connection_id, request_id)).await
      };
      if let Some(sub) = sub_opt {
        return Some((track_arc, sub));
      }
    }
    None
  }

  pub async fn has_track_alias(&self, connection_id: usize, track_alias: &u64) -> bool {
    let track_aliases = self.track_aliases.read().await;
    track_aliases.contains_key(&(connection_id, *track_alias))
  }

  /// Clears the alias and the `publishes` entry of a publisher, without removing
  /// the track itself. Used on every PUBLISH_DONE and when a publishing session
  /// closes. The caller must already have taken the publisher out of its track
  /// with `Track::remove_publisher`, which returns the alias to pass here.
  ///
  /// It does not take `tracks`, so a caller may hold it.
  pub async fn remove_publisher_alias(&self, connection_id: usize, track_alias: u64) {
    let mut track_aliases = self.track_aliases.write().await;
    let track_name_opt = track_aliases.remove(&(connection_id, track_alias));
    if let Some(track_name) = track_name_opt {
      info!(
        "Removed publisher alias {}@{} from track_aliases, track: {}",
        track_alias, connection_id, track_name
      );

      // remove the track alias from the publishes
      let mut publishes = self.publishes.write().await;
      if let Some(map) = publishes.get_mut(&track_name) {
        map.remove(&connection_id);
      }
    }
  }

  /// Atomically gets an existing track or creates a new one.
  /// The factory receives the generated relay_track_id and must use it when constructing Track.
  /// Returns the track Arc and a boolean indicating whether this call created the track.
  pub async fn get_or_create_track(
    &self,
    full_track_name: &FullTrackName,
    track_factory: impl FnOnce(u64) -> Track,
  ) -> (Arc<Track>, bool) {
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
      let track_arc = Arc::new(track);
      tracks.insert(full_track_name.clone(), track_arc.clone());
      // Do NOT insert into track_aliases -- the publisher alias is
      // unknown until SubscribeOk arrives. Use add_track_alias() later.
      (track_arc, true)
    }
  }

  /// Register a track alias mapping. Called when the publisher's SubscribeOk
  /// reveals the actual track_alias, or when a publisher registers via Publish.
  #[must_use]
  pub async fn add_track_alias(
    &self,
    connection_id: usize,
    track_alias: u64,
    full_track_name: FullTrackName,
  ) -> bool {
    {
      let mut track_aliases = self.track_aliases.write().await;
      match track_aliases.entry((connection_id, track_alias)) {
        Entry::Occupied(existing) if *existing.get() != full_track_name => {
          warn!(
            "Track alias {}@{} already maps to {:?}; refusing {:?}",
            track_alias,
            connection_id,
            existing.get(),
            full_track_name
          );
          return false;
        }
        Entry::Occupied(_) => {}
        Entry::Vacant(slot) => {
          slot.insert(full_track_name.clone());
        }
      }
    }
    self.alias_registered.notify_waiters();
    info!(
      "Registered track alias {}@{} -> {:?}",
      track_alias, connection_id, full_track_name
    );
    true
  }

  /// Resolves an alias to its track, waiting up to `timeout` for the control message
  /// that establishes it. Data streams can outrun that message, which the specification
  /// anticipates: a receiver may buffer briefly before abandoning the stream.
  pub async fn resolve_track_by_alias(
    &self,
    connection_id: usize,
    track_alias: u64,
    timeout: Duration,
  ) -> Option<Arc<Track>> {
    let deadline = Instant::now() + timeout;
    loop {
      // Subscribe before looking, so a registration between the two is not missed.
      let registered = self.alias_registered.notified();

      let full_track_name = {
        let aliases = self.track_aliases.read().await;
        aliases.get(&(connection_id, track_alias)).cloned()
      };
      if let Some(full_track_name) = full_track_name {
        return self.get_track(&full_track_name).await;
      }

      let remaining = deadline.saturating_duration_since(Instant::now());
      if remaining.is_zero() || tokio::time::timeout(remaining, registered).await.is_err() {
        return None;
      }
    }
  }

  pub async fn add_namespace_subscriber(
    &self,
    prefix: Tuple,
    client: Arc<MOQTClient>,
    kind: SubscribeKind,
    parameters: Vec<MessageParameter>,
    namespace_tx: UnboundedSender<ControlMessage>,
  ) {
    let mut subs = self.namespace_subscribers.write().await;

    // Get or create the list for this prefix
    let clients = subs.entry(prefix.clone()).or_insert_with(Vec::new);

    // Avoid duplicates of the same client and kind
    if !clients
      .iter()
      .any(|(c, k, _, _)| c.connection_id == client.connection_id && *k == kind)
    {
      clients.push((client, kind, parameters, namespace_tx));
    }
    info!("Added namespace subscriber for prefix {:?}", prefix);
  }

  /// Clients of the given kind whose prefix matches the target namespace.
  pub async fn get_namespace_subscribers(
    &self,
    target_namespace: &Tuple,
    kind: SubscribeKind,
  ) -> Vec<(Arc<MOQTClient>, Vec<MessageParameter>)> {
    let subs = self.namespace_subscribers.read().await;
    let mut interested_clients = Vec::new();

    // Check every prefix. If target starts with prefix, they are interested.
    // Example: Target "meet.room1", Prefix "meet" -> Match.
    for (prefix, clients) in subs.iter() {
      if target_namespace.starts_with(prefix) {
        for (client, k, params, _tx) in clients {
          if *k == kind {
            // The parameters come along: what the relay sends a subscriber is derived
            // from the request that asked for it, not from the publisher upstream.
            interested_clients.push((client.clone(), params.clone()));
          }
        }
      }
    }
    interested_clients
  }

  /// Updates the stored PublishNamespace message with new parameters.
  /// Ensures new subscribers to this namespace get updated Auth/Metadata.
  pub async fn update_namespace_parameters(
    &self,
    namespace: &Tuple,
    new_parameters: Vec<moqtail::model::parameter::message_parameter::MessageParameter>,
  ) {
    let mut announcements = self.announcements.write().await;
    if let Some((_client, message)) = announcements.get_mut(namespace) {
      info!(
        "Updating stored Announcement parameters for {:?}",
        namespace
      );
      apply_message_parameter_update(&mut message.parameters, new_parameters);
    }
  }

  pub async fn add_announcement(
    &self,
    namespace: Tuple,
    publisher: Arc<MOQTClient>,
    message: PublishNamespace,
  ) {
    let mut announcements = self.announcements.write().await;
    announcements.insert(namespace.clone(), (publisher, message));
    info!("Stored announcement for namespace: {:?}", namespace);
  }

  /// Drops the announcement only if this connection is the one that made it.
  pub async fn remove_announcement(&self, namespace: &Tuple, connection_id: usize) -> bool {
    let mut announcements = self.announcements.write().await;
    match announcements.get(namespace) {
      Some((client, _message)) if client.connection_id == connection_id => {
        announcements.remove(namespace);
        info!(
          "Removed announcement for namespace {:?} (publisher {})",
          namespace, connection_id
        );
        true
      }
      _ => false,
    }
  }

  /// Returns what it removed, so the caller can tell the subscribers that heard about them.
  pub async fn remove_announcements_by_connection(&self, connection_id: usize) -> Vec<Tuple> {
    let mut announcements = self.announcements.write().await;
    let mut removed = Vec::new();
    announcements.retain(|ns, (client, _message)| {
      if client.connection_id == connection_id {
        info!(
          "Removed announcement for namespace {:?} (publisher {} disconnected)",
          ns, connection_id
        );
        removed.push(ns.clone());
        false
      } else {
        true
      }
    });
    removed
  }

  pub async fn remove_namespace_subscriber(&self, connection_id: usize) {
    let mut subs = self.namespace_subscribers.write().await;
    for clients in subs.values_mut() {
      clients.retain(|(c, _, _, _)| c.connection_id != connection_id);
    }
    subs.retain(|_, clients| !clients.is_empty());
  }

  pub async fn remove_namespace_subscriber_by_prefix(
    &self,
    prefix: &Tuple,
    connection_id: usize,
    kind: SubscribeKind,
  ) {
    let mut subs = self.namespace_subscribers.write().await;
    if let Some(clients) = subs.get_mut(prefix) {
      clients.retain(|(c, k, _, _)| !(c.connection_id == connection_id && *k == kind));
    }
    subs.retain(|_, clients| !clients.is_empty());
  }

  pub async fn get_announcements_by_prefix(&self, prefix: &Tuple) -> Vec<Tuple> {
    let announcements = self.announcements.read().await;
    let mut matches = Vec::new();
    for ns in announcements.keys() {
      if ns.starts_with(prefix) {
        matches.push(ns.clone());
      }
    }
    matches
  }

  pub async fn update_namespace_subscription_parameters(
    &self,
    prefix: &Tuple,
    connection_id: usize,
    new_parameters: Vec<moqtail::model::parameter::message_parameter::MessageParameter>,
  ) {
    let mut subs = self.namespace_subscribers.write().await;
    if let Some(clients) = subs.get_mut(prefix)
      && let Some((_client, _kind, params, _tx)) = clients
        .iter_mut()
        .find(|(c, _, _, _)| c.connection_id == connection_id)
    {
      apply_message_parameter_update(params, new_parameters);
    }
  }

  pub async fn add_publish_message(
    &self,
    full_track_name: FullTrackName,
    connection_id: usize,
    publish_msg: Publish,
  ) {
    let mut publishes = self.publishes.write().await;
    if let Some(map) = publishes.get_mut(&full_track_name) {
      map.insert(connection_id, publish_msg);
    } else {
      let mut map: HashMap<usize, Publish> = HashMap::new();
      map.insert(connection_id, publish_msg);
      publishes.insert(full_track_name, map);
    }
  }

  /// The request id of the PUBLISH this connection used for the track, which
  /// identifies the request stream its responses belong on.
  pub async fn get_publish_request_id(
    &self,
    full_track_name: &FullTrackName,
    connection_id: usize,
  ) -> Option<u64> {
    let publishes = self.publishes.read().await;
    publishes
      .get(full_track_name)
      .and_then(|by_connection| by_connection.get(&connection_id))
      .map(|publish| publish.request_id)
  }

  pub async fn get_tracks_and_publishes_by_namespace_prefix(
    &self,
    prefix: &Tuple,
  ) -> Vec<(FullTrackName, Arc<Track>, Option<Publish>)> {
    let tracks = self.tracks.read().await;
    let publishes = self.publishes.read().await;
    let mut matches = Vec::new();

    for (full_track_name, track_arc) in tracks.iter() {
      let is_match = full_track_name.namespace.fields.starts_with(&prefix.fields);
      trace!(
        "checking track: {} against prefix.fields: {:?} is_match: {}",
        full_track_name, prefix.fields, is_match
      );

      if is_match && let Some(pub_msg_map) = publishes.get(full_track_name) {
        // return the first publish message
        matches.push((
          full_track_name.clone(),
          track_arc.clone(),
          pub_msg_map.values().next().cloned(),
        ));
      } else if is_match {
        warn!("No publish for track {}", full_track_name);
      }
    }
    matches
  }

  /// Returns the first existing namespace subscription prefix for `connection_id`
  /// that overlaps with `new_prefix` (equal, or one is a prefix of the other).
  /// Returns `None` if no overlap is found.
  /// A prefix already subscribed by this connection with the same kind that
  /// overlaps the new prefix. Overlap spaces are independent per kind, so a
  /// SUBSCRIBE_NAMESPACE never conflicts with a SUBSCRIBE_TRACKS.
  ///
  /// `exclude` names a prefix to skip, which is how a subscription being moved
  /// avoids conflicting with the entry it is about to vacate.
  pub async fn find_overlapping_namespace_subscription(
    &self,
    connection_id: usize,
    new_prefix: &Tuple,
    kind: SubscribeKind,
    exclude: Option<&Tuple>,
  ) -> Option<Tuple> {
    let subs = self.namespace_subscribers.read().await;
    for (existing_prefix, clients) in subs.iter() {
      if exclude == Some(existing_prefix) {
        continue;
      }
      if clients
        .iter()
        .any(|(c, k, _, _)| c.connection_id == connection_id && *k == kind)
        && namespace_prefixes_overlap(new_prefix, existing_prefix)
      {
        return Some(existing_prefix.clone());
      }
    }
    None
  }

  /// Moves this connection's prefix subscription of `kind` from `old_prefix` to
  /// `new_prefix`, carrying its parameters and channel across so the subscriber
  /// keeps receiving on the stream it already has. Returns false if there was no
  /// such subscription to move.
  pub async fn rekey_namespace_subscriber(
    &self,
    old_prefix: &Tuple,
    new_prefix: Tuple,
    connection_id: usize,
    kind: SubscribeKind,
  ) -> bool {
    let mut subs = self.namespace_subscribers.write().await;

    let Some(clients) = subs.get_mut(old_prefix) else {
      return false;
    };
    let Some(position) = clients
      .iter()
      .position(|(c, k, _, _)| c.connection_id == connection_id && *k == kind)
    else {
      return false;
    };
    let subscriber = clients.remove(position);
    if clients.is_empty() {
      subs.remove(old_prefix);
    }

    subs.entry(new_prefix).or_default().push(subscriber);
    true
  }

  pub async fn get_track_name_by_publisher(
    &self,
    connection_id: usize,
    request_id: u64,
  ) -> Option<FullTrackName> {
    // 1. Find the tracks published with this Request ID. Per the lock order, the
    // maps are released before any track is used below.
    let candidates: Vec<(FullTrackName, Arc<Track>)> = {
      let tracks = self.tracks.read().await;
      let publishes = self.publishes.read().await;
      publishes
        .iter()
        .filter(|(_, by_connection)| {
          by_connection
            .get(&connection_id)
            .is_some_and(|publish_msg| publish_msg.request_id == request_id)
        })
        .filter_map(|(track_name, _)| {
          tracks
            .get(track_name)
            .map(|track_arc| (track_name.clone(), track_arc.clone()))
        })
        .collect()
    };

    for (track_name, track_arc) in candidates {
      // 2. Verify that the client sending the REQUEST_UPDATE is actually the owner!
      // TODO: do we really need this check? Look at this later.
      let track = &track_arc;
      if track
        .publisher_aliases
        .read()
        .await
        .contains_key(&connection_id)
      {
        return Some(track_name);
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
  use crate::server::config::{AppConfig, Cli};
  use crate::server::track::{TrackOrigin, TrackStatus};
  use bytes::Bytes;
  use clap::Parser;
  use moqtail::model::data::constant::ObjectForwardingPreference;
  use moqtail::model::data::fetch_object::FetchObjectPayload;

  fn t(path: &str) -> Tuple {
    Tuple::from_utf8_path(path)
  }

  /// The relay's own defaults, leaked because Track borrows its config for 'static.
  fn config() -> &'static AppConfig {
    Box::leak(Box::new(AppConfig::from_cli(Cli::parse_from(["relay"]))))
  }

  fn track_name() -> FullTrackName {
    FullTrackName::try_new("meet/room1", "video").unwrap()
  }

  /// Takes or creates the track, then registers the alias its data streams arrive
  /// under, the way the PUBLISH handler does.
  async fn publish(
    manager: &TrackManager,
    connection_id: usize,
    track_alias: u64,
    full_track_name: &FullTrackName,
  ) -> (Arc<Track>, bool) {
    let (track, created) = manager
      .get_or_create_track(full_track_name, |relay_track_id| {
        Track::new(
          relay_track_id,
          full_track_name.clone(),
          config(),
          TrackStatus::Confirmed {
            upstream_parameters: vec![],
          },
          TrackOrigin::Publish,
        )
      })
      .await;
    assert!(
      manager
        .add_track_alias(connection_id, track_alias, full_track_name.clone())
        .await
    );
    (track, created)
  }

  #[tokio::test]
  async fn a_second_publisher_joins_the_track_instead_of_replacing_it() {
    let manager = TrackManager::new();
    let name = track_name();

    let (first, first_created) = publish(&manager, 1, 10, &name).await;
    let (second, second_created) = publish(&manager, 2, 20, &name).await;

    assert!(Arc::ptr_eq(&first, &second));
    // Only the first publisher does a first publisher's work.
    assert!(first_created);
    assert!(!second_created);
    assert_eq!(manager.tracks.read().await.len(), 1);
    // Both aliases reach that one track.
    for (connection_id, alias) in [(1usize, 10u64), (2, 20)] {
      let resolved = manager
        .get_track_by_alias(connection_id, alias)
        .await
        .expect("alias resolves");
      assert!(Arc::ptr_eq(&resolved, &first));
    }
  }

  #[tokio::test]
  async fn one_publisher_finishing_leaves_the_other_publishing() {
    let manager = TrackManager::new();
    let name = track_name();

    let (first, _) = publish(&manager, 1, 10, &name).await;
    first.add_publisher(1, 10).await;
    let (second, _) = publish(&manager, 2, 20, &name).await;
    second.add_publisher(2, 20).await;

    // PUBLISH_DONE from the second publisher, resolved through the manager the way
    // cleanup_published_track does.
    let track = manager.get_track(&name).await.expect("track is registered");
    let removal = track
      .remove_publisher(2)
      .await
      .expect("publisher 2 was registered");
    manager.remove_publisher_alias(2, removal.alias).await;
    // What the caller branches on: the track is thinner, not finished. Everything
    // keyed to the track outliving this PUBLISH_DONE hangs off this flag --
    // dropping it from its subscribers' switching sets among them, which would
    // stop it being gated and let every rendition of its set forward at once.
    assert!(
      removal.still_served,
      "publisher 1 is still serving, so the track is not over"
    );
    if !removal.still_served {
      manager.remove_track(&name).await;
    }

    // The first publisher is still sending.
    assert!(manager.get_track(&name).await.is_some());
    assert!(manager.get_track_by_alias(1, 10).await.is_some());
    assert!(manager.get_track_by_alias(2, 20).await.is_none());

    // And when the last one goes, the removal says so.
    let removal = track
      .remove_publisher(1)
      .await
      .expect("publisher 1 was registered");
    assert!(!removal.still_served, "no publishers remain");
  }

  #[tokio::test]
  async fn one_publishers_publish_done_does_not_end_a_track_another_still_serves() {
    // A PUBLISH_DONE says its sender stopped, for whatever reason of its own.
    // While another publisher serves the track the subscribers must not be told
    // it ended, so the upstream PUBLISH_DONE path asks this before relaying it.
    let manager = TrackManager::new();
    let name = track_name();

    let (first, _) = publish(&manager, 1, 10, &name).await;
    first.add_publisher(1, 10).await;
    let (second, _) = publish(&manager, 2, 20, &name).await;
    second.add_publisher(2, 20).await;

    let track = manager.get_track(&name).await.expect("track is registered");

    assert!(
      track.remove_publisher_quietly(2).await,
      "publisher 1 still serves the track"
    );
    assert!(
      !track.remove_publisher_quietly(1).await,
      "the last publisher leaving ends the track"
    );
  }

  #[tokio::test]
  async fn removing_an_unregistered_publisher_still_answers_for_the_others() {
    // Only the PUBLISH path registers a publisher, so a relay that subscribed
    // upstream may hold no alias for the connection sending PUBLISH_DONE. The
    // answer has to be about the track rather than about the removal.
    let manager = TrackManager::new();
    let name = track_name();

    let (first, _) = publish(&manager, 1, 10, &name).await;
    first.add_publisher(1, 10).await;

    let track = manager.get_track(&name).await.expect("track is registered");
    assert!(
      track.remove_publisher_quietly(99).await,
      "publisher 1 serves the track, whatever connection 99 was"
    );
  }

  #[tokio::test]
  async fn a_parked_data_stream_is_woken_when_its_publisher_registers() {
    // A data stream can outrun the PUBLISH that names its alias.
    let manager = TrackManager::new();
    let name = track_name();

    let resolver = {
      let manager = manager.clone();
      tokio::spawn(async move {
        manager
          .resolve_track_by_alias(1, 10, Duration::from_secs(30))
          .await
      })
    };
    tokio::task::yield_now().await;

    publish(&manager, 1, 10, &name).await;

    let resolved = tokio::time::timeout(Duration::from_secs(5), resolver)
      .await
      .expect("the waiter is woken by the registration, not by its timeout")
      .expect("resolver task");
    assert!(resolved.is_some());
  }

  /// Yields enough times for every spawned task to reach its first wait. This
  /// orders the tasks only on tokio's single-thread test runtime, which these
  /// tests use.
  async fn settle() {
    for _ in 0..10 {
      tokio::task::yield_now().await;
    }
  }

  /// Spawns `remove_track(name)` and lets it run until it holds, or waits for,
  /// the tracks map for writing.
  async fn spawn_remove_track(
    manager: &TrackManager,
    name: &FullTrackName,
  ) -> tokio::task::JoinHandle<()> {
    let remover = {
      let manager = manager.clone();
      let name = name.clone();
      tokio::spawn(async move { manager.remove_track(&name).await })
    };
    settle().await;
    assert!(
      manager.tracks.try_read().is_err(),
      "remove_track holds or waits for the tracks map"
    );
    remover
  }

  #[tokio::test]
  async fn a_publisher_alias_is_removed_under_the_tracks_lock_while_a_writer_waits() {
    // A caller may hold the tracks map while a track removal waits to write it. The
    // waiting writer blocks new readers, so removing an alias must not take the map.
    let manager = TrackManager::new();
    let name = track_name();
    let other = FullTrackName::try_new("meet/room2", "video").unwrap();
    let (track, _) = publish(&manager, 1, 10, &name).await;
    track.add_publisher(1, 10).await;
    publish(&manager, 2, 20, &other).await;

    let tracks = manager.tracks.read().await;
    let remover = spawn_remove_track(&manager, &other).await;

    let removal = track
      .remove_publisher(1)
      .await
      .expect("publisher 1 was registered");
    tokio::time::timeout(
      Duration::from_secs(1),
      manager.remove_publisher_alias(1, removal.alias),
    )
    .await
    .expect("removing the alias does not take the tracks map");
    assert!(!manager.has_track_alias(1, &10).await);

    drop(tracks);
    tokio::time::timeout(Duration::from_secs(1), remover)
      .await
      .expect("the other removal proceeds once the walk is done")
      .expect("remover task");
  }

  #[tokio::test]
  async fn a_closing_session_does_not_hold_the_tracks_map_while_it_visits_tracks() {
    // Cleanup waits on a busy track; creating another track must not wait with it.
    let manager = TrackManager::new();
    let name = track_name();
    let (track, _) = publish(&manager, 1, 10, &name).await;
    track.add_publisher(1, 10).await;

    let busy = track.publisher_aliases.write().await;
    let closing = {
      let manager = manager.clone();
      tokio::spawn(async move { manager.remove_publisher_everywhere(1).await })
    };
    settle().await;
    assert!(
      !closing.is_finished(),
      "the session waits for the busy track"
    );

    let other = FullTrackName::try_new("meet/room2", "video").unwrap();
    tokio::time::timeout(Duration::from_secs(1), publish(&manager, 2, 20, &other))
      .await
      .expect("a track is created while the session waits");

    drop(busy);
    let unserved = tokio::time::timeout(Duration::from_secs(1), closing)
      .await
      .expect("the session finishes once the track is free")
      .expect("closing task");
    assert_eq!(unserved, vec![name]);
    assert!(!manager.has_track_alias(1, &10).await);
  }

  #[tokio::test]
  async fn an_alias_lookup_does_not_deadlock_with_a_track_removal() {
    // remove_track clears the removed track's aliases while it still holds the
    // tracks map; a lookup by alias must not hold the aliases while it waits for
    // that map.
    let manager = TrackManager::new();
    let name = track_name();
    let other = FullTrackName::try_new("meet/room2", "video").unwrap();
    let (track, _) = publish(&manager, 1, 10, &name).await;
    // remove_track takes track_aliases only for a track with publisher aliases.
    track.add_publisher(1, 10).await;
    publish(&manager, 2, 20, &other).await;

    // Parks remove_track after it has taken the tracks map.
    let parked = track.publisher_aliases.write().await;
    let remover = spawn_remove_track(&manager, &name).await;
    let lookup = {
      let manager = manager.clone();
      tokio::spawn(async move { manager.get_track_by_alias(2, 20).await })
    };
    settle().await;
    assert!(!lookup.is_finished(), "the lookup waits for a lock");
    drop(parked);

    let ((), found) = tokio::time::timeout(Duration::from_secs(1), async {
      tokio::try_join!(remover, lookup)
    })
    .await
    .expect("the removal and the lookup both finish")
    .expect("tasks");
    assert!(found.is_some());
    assert!(!manager.has_track_alias(1, &10).await);
  }

  #[tokio::test]
  async fn a_publisher_lookup_does_not_deadlock_with_a_track_removal() {
    // remove_track clears the removed track's publishes while it still holds the
    // tracks map; finding a publisher's track must not hold the publishes while it
    // waits for that map. The lookup takes the publishes even when nothing matches.
    let manager = TrackManager::new();
    let name = track_name();
    let (track, _) = publish(&manager, 1, 10, &name).await;

    // Parks remove_track after it has taken the tracks map.
    let parked = track.publisher_aliases.write().await;
    let remover = spawn_remove_track(&manager, &name).await;
    let lookup = {
      let manager = manager.clone();
      tokio::spawn(async move { manager.get_track_name_by_publisher(2, 0).await })
    };
    settle().await;
    assert!(!lookup.is_finished(), "the lookup waits for a lock");
    drop(parked);

    let ((), found) = tokio::time::timeout(Duration::from_secs(1), async {
      tokio::try_join!(remover, lookup)
    })
    .await
    .expect("the removal and the lookup both finish")
    .expect("tasks");
    assert!(found.is_none());
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

  #[tokio::test]
  async fn an_alias_already_used_for_another_track_is_refused() {
    let manager = TrackManager::new();
    let a = FullTrackName::try_new("meet/room1", "a").unwrap();
    let b = FullTrackName::try_new("meet/room1", "b").unwrap();

    assert!(manager.add_track_alias(1, 7, a.clone()).await);
    assert!(!manager.add_track_alias(1, 7, b).await);

    // The first track keeps the alias.
    assert_eq!(manager.track_aliases.read().await.get(&(1, 7)), Some(&a));
  }

  #[tokio::test]
  async fn an_alias_may_be_registered_again_for_the_same_track() {
    let manager = TrackManager::new();
    let name = track_name();

    assert!(manager.add_track_alias(1, 7, name.clone()).await);
    assert!(manager.add_track_alias(1, 7, name).await);
  }

  #[tokio::test]
  async fn the_same_alias_from_different_publishers_does_not_collide() {
    let manager = TrackManager::new();
    let a = FullTrackName::try_new("meet/room1", "a").unwrap();
    let b = FullTrackName::try_new("meet/room1", "b").unwrap();

    // Aliases are scoped to the session that chose them.
    assert!(manager.add_track_alias(1, 7, a).await);
    assert!(manager.add_track_alias(2, 7, b).await);
  }

  /// Takes or creates the track the way the FETCH handler does.
  async fn fetch(manager: &TrackManager, full_track_name: &FullTrackName) -> Arc<Track> {
    let (track, _) = manager
      .get_or_create_track(full_track_name, |relay_track_id| {
        Track::new(
          relay_track_id,
          full_track_name.clone(),
          config(),
          TrackStatus::Pending,
          TrackOrigin::CacheOnly,
        )
      })
      .await;
    track
  }

  #[tokio::test]
  async fn a_sweep_removes_only_empty_cache_only_tracks_nothing_holds() {
    let manager = TrackManager::new();
    let name = |n: &str| FullTrackName::try_new("meet/room1", n).unwrap();

    fetch(&manager, &name("empty")).await;
    let held = fetch(&manager, &name("held")).await;
    let cached = fetch(&manager, &name("cached")).await;
    cached
      .cache
      .add_object(FetchObjectPayload {
        group_id: 0,
        subgroup_id: 0,
        object_id: 0,
        publisher_priority: 0,
        forwarding_preference: ObjectForwardingPreference::Subgroup,
        properties: None,
        payload: Bytes::from_static(b"x"),
      })
      .await;
    drop(cached);
    publish(&manager, 1, 7, &name("published")).await;

    assert_eq!(manager.sweep_empty_cache_only_tracks().await, 1);
    assert!(manager.get_track(&name("empty")).await.is_none());
    for kept in ["held", "cached", "published"] {
      assert!(manager.get_track(&name(kept)).await.is_some(), "{kept}");
    }

    drop(held);
    assert_eq!(manager.sweep_empty_cache_only_tracks().await, 1);
    assert!(manager.get_track(&name("held")).await.is_none());
  }

  #[tokio::test]
  async fn the_first_subscribe_takes_over_a_cache_only_track() {
    let manager = TrackManager::new();
    let name = track_name();
    let fetched = fetch(&manager, &name).await;

    // Only the first one takes it, and so only it forwards the SUBSCRIBE upstream.
    for (n, expected) in [(1, true), (2, false)] {
      let (track, created) = manager
        .get_or_create_track(&name, |_| unreachable!("the track exists"))
        .await;
      assert!(!created);
      let taken_over = track
        .take_over(TrackOrigin::Subscribe, TrackStatus::Pending)
        .await;
      assert_eq!(taken_over, expected, "subscribe {n}");
    }

    let track = &fetched;
    assert_eq!(track.origin(), TrackOrigin::Subscribe);
    assert!(matches!(
      track.status.read().await.clone(),
      TrackStatus::Pending
    ));
  }

  #[tokio::test]
  async fn a_publish_takes_over_a_fetched_track_and_confirms_it() {
    let manager = TrackManager::new();
    let name = track_name();
    let fetched = fetch(&manager, &name).await;

    let (track, created) = publish(&manager, 1, 7, &name).await;
    assert!(!created);
    assert!(
      Arc::ptr_eq(&track, &fetched),
      "the cache comes with the track"
    );
    assert!(
      track
        .take_over(
          TrackOrigin::Publish,
          TrackStatus::Confirmed {
            upstream_parameters: vec![],
          },
        )
        .await
    );

    assert_eq!(track.origin(), TrackOrigin::Publish);
    assert!(matches!(
      track.status.read().await.clone(),
      TrackStatus::Confirmed { .. }
    ));
  }

  #[tokio::test]
  async fn a_track_that_is_not_cache_only_is_never_taken_over() {
    let manager = TrackManager::new();
    let name = track_name();
    let (track, _) = publish(&manager, 1, 7, &name).await;

    assert!(
      !track
        .take_over(TrackOrigin::Subscribe, TrackStatus::Pending)
        .await
    );
    assert_eq!(track.origin(), TrackOrigin::Publish);
  }
}
