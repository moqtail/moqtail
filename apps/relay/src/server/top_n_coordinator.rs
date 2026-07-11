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

//! Top-N active-speaker coordinator.
//!
//! Manages a periodic re-rank tick that determines which Subscriptions exist for
//! namespace-subscribers that sent a TrackFilter (0x12) parameter. Between ticks
//! the object hot path is untouched. Tracks that never emit the ranked property
//! extension (e.g. video) are always subscribed unconditionally.
//!
//! We use dwell debounce to avoid flapping subscriptions when a track is on the edge of the top-N list.
//! We don't instantly remove a track from the top list the millisecond it drops out.
//! Wait and make sure it stays out for several consecutive updates first to ensure
//! the drop is real and permanent.

use crate::server::client::MOQTClient;
use crate::server::config::AppConfig;
use crate::server::session_context::PendingRequest;
use crate::server::track_manager::TrackManager;
use moqtail::model::common::pair::KeyValuePair;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::extension_header::object_extension::ObjectExtension;
use moqtail::model::filter::top_n::{TieBreakPolicy, TopNRanker, TopNRankerConfig};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Speech-activity extension key (moq-transport PR #1401 custom extension).
const SPEECH_ACTIVITY_KEY: u64 = 0x12;

// type aliases for readability
type SubscriberKey = (Tuple, usize); // (namespace_prefix, subscriber_connection_id)
type SubscriberMap = HashMap<SubscriberKey, SubscriberFilterState>;
type RankerKey = (Tuple, u64); // (namespace_prefix, property_type)
type RankerMap = HashMap<RankerKey, TopNRanker>;
type PendingRequestMap = BTreeMap<u64, PendingRequest>; // request_id -> pending request

/// Per-subscriber state for Top-N filtering.
struct SubscriberFilterState {
  client: Arc<MOQTClient>,
  property_type: u64,
  n: usize,
  /// Tracks currently subscribed via the coordinator (not unconditional pass-throughs).
  current_selected: HashSet<FullTrackName>,
  /// How many consecutive ticks each track has ranked outside top-N (for dwell debounce).
  pending_removal_streak: HashMap<FullTrackName, u32>,
  /// True if this connection also publishes tracks (panelist). False for audience-only.
  /// Audience subscribers share identical top-N results (no self-exclusion needed).
  is_publisher: bool,
}

pub struct TopNCoordinator {
  track_manager: TrackManager,
  relay_next_request_id: Arc<AtomicU64>,
  relay_pending_requests: Arc<RwLock<PendingRequestMap>>,
  rankers: Arc<RwLock<RankerMap>>,
  subscribers: Arc<RwLock<SubscriberMap>>,
  update_seq: Arc<AtomicU64>,
  config: &'static AppConfig,
}

impl std::fmt::Debug for TopNCoordinator {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("TopNCoordinator").finish_non_exhaustive()
  }
}

impl TopNCoordinator {
  pub fn new(
    track_manager: TrackManager,
    relay_next_request_id: Arc<AtomicU64>,
    relay_pending_requests: Arc<RwLock<PendingRequestMap>>,
    config: &'static AppConfig,
  ) -> Self {
    Self {
      track_manager,
      relay_next_request_id,
      relay_pending_requests,
      rankers: Arc::new(RwLock::new(RankerMap::new())),
      subscribers: Arc::new(RwLock::new(SubscriberMap::new())),
      update_seq: Arc::new(AtomicU64::new(0)),
      config,
    }
  }

  /// Register a subscriber's Top-N filter request.
  /// Runs an immediate first reconciliation: tracks that are already ranked are gated;
  /// unranked tracks get unconditional pass-through subscriptions (handled by caller).
  pub async fn register_subscriber(
    &self,
    prefix: Tuple,
    client: Arc<MOQTClient>,
    property_type: u64,
    n: usize,
    is_publisher: bool,
  ) {
    // Create ranker if not already present for this (prefix, property_type) pair.
    {
      let mut rankers = self.rankers.write().await;
      rankers
        .entry((prefix.clone(), property_type))
        .or_insert_with(|| {
          TopNRanker::new(TopNRankerConfig {
            property_type,
            tie_break_policy: TieBreakPolicy::OldestWins,
          })
        });
    }

    let conn_id = client.connection_id;
    let state = SubscriberFilterState {
      client,
      property_type,
      n,
      current_selected: HashSet::new(),
      pending_removal_streak: HashMap::new(),
      is_publisher,
    };

    self
      .subscribers
      .write()
      .await
      .insert((prefix.clone(), conn_id), state);

    info!(
      "TopNCoordinator: registered subscriber conn={} prefix={:?} property_type=0x{:02X} n={}",
      conn_id, prefix, property_type, n
    );
  }

  /// Update N or property_type for an existing subscriber (RequestUpdate).
  pub async fn update_subscriber(
    &self,
    prefix: &Tuple,
    connection_id: usize,
    property_type: u64,
    n: usize,
  ) {
    let mut subs = self.subscribers.write().await;
    if let Some(state) = subs.get_mut(&(prefix.clone(), connection_id)) {
      state.property_type = property_type;
      state.n = n;
      info!(
        "TopNCoordinator: updated subscriber conn={} prefix={:?} n={}",
        connection_id, prefix, n
      );
    }
  }

  /// Remove a subscriber's registration (cancel or graceful disconnect).
  pub async fn remove_subscriber(&self, prefix: &Tuple, connection_id: usize) {
    let removed = self
      .subscribers
      .write()
      .await
      .remove(&(prefix.clone(), connection_id));
    if removed.is_some() {
      info!(
        "TopNCoordinator: removed subscriber conn={} prefix={:?}",
        connection_id, prefix
      );
    }
    self.gc_empty_rankers().await;
  }

  /// Remove all registrations for a connection (ungraceful disconnect).
  pub async fn remove_subscriber_by_connection(&self, connection_id: usize) {
    self
      .subscribers
      .write()
      .await
      .retain(|(_, conn), _| *conn != connection_id);
    self.gc_empty_rankers().await;
    info!(
      "TopNCoordinator: removed all state for conn={}",
      connection_id
    );
  }

  /// Check whether a subscriber connection has a TopN filter registered for a given namespace prefix.
  pub async fn is_top_n_subscriber(&self, prefix: &Tuple, connection_id: usize) -> bool {
    let subs = self.subscribers.read().await;
    subs.contains_key(&(prefix.clone(), connection_id))
  }

  /// Returns the top-N track names for a subscriber at the moment of the call.
  /// Returns None if the subscriber has no registered filter for this prefix.
  pub async fn compute_top_n_for(
    &self,
    prefix: &Tuple,
    connection_id: usize,
  ) -> Option<Vec<FullTrackName>> {
    let subs = self.subscribers.read().await;
    let state = subs.get(&(prefix.clone(), connection_id))?;
    let property_type = state.property_type;
    let n = state.n;
    drop(subs);

    let rankers = self.rankers.read().await;
    let ranker = rankers.get(&(prefix.clone(), property_type))?;
    Some(ranker.compute_top_n(connection_id, n))
  }

  /// Returns the property_type configured by a subscriber's TrackFilter, if any.
  pub async fn get_subscriber_property_type(
    &self,
    prefix: &Tuple,
    connection_id: usize,
  ) -> Option<u64> {
    let subs = self.subscribers.read().await;
    subs
      .get(&(prefix.clone(), connection_id))
      .map(|s| s.property_type)
  }

  /// Returns whether the ranker has an entry for this track (i.e. it has been observed
  /// emitting the property extension). Used by subscribe_namespace_handler to distinguish
  /// pass-through (unranked) vs. top-N gated tracks.
  pub async fn ranker_has_entry(
    &self,
    prefix: &Tuple,
    property_type: u64,
    full_track_name: &FullTrackName,
  ) -> bool {
    let rankers = self.rankers.read().await;
    if let Some(ranker) = rankers.get(&(prefix.clone(), property_type)) {
      ranker.has_entry(full_track_name)
    } else {
      false
    }
  }

  /// Hot-path: scan object extensions for the configured property key and update the ranker.
  /// Returns immediately (no lock) if the key is absent — this is the common case.
  pub async fn observe_object_extension(
    &self,
    full_track_name: &FullTrackName,
    publisher_conn_ids: Vec<usize>,
    extensions: &[ObjectExtension],
  ) {
    let value = match extract_varint_extension(extensions, SPEECH_ACTIVITY_KEY) {
      Some(v) => v,
      None => return,
    };

    let update_seq = self.update_seq.fetch_add(1, Ordering::Relaxed);
    let mut rankers = self.rankers.write().await;
    let namespace = &full_track_name.namespace;

    for ((prefix, pt), ranker) in rankers.iter_mut() {
      if *pt == SPEECH_ACTIVITY_KEY && namespace.starts_with(prefix) {
        ranker.observe_value(
          full_track_name,
          publisher_conn_ids.clone(),
          value,
          update_seq,
        );
      }
    }
  }

  /// Called when a publisher disconnects. Removes its entry from all rankers immediately.
  /// Subscription teardown happens on the next tick.
  pub async fn on_publisher_disconnected(&self, connection_id: usize) {
    let mut rankers = self.rankers.write().await;
    for ranker in rankers.values_mut() {
      ranker.remove_publisher(connection_id);
    }
  }

  /// Spawn the periodic tick loop. Call once from Server::start().
  pub fn spawn_tick_loop(self: Arc<Self>) {
    tokio::spawn(async move {
      let interval = self.config.top_n_tick_interval;
      loop {
        tokio::time::sleep(interval).await;
        self.tick().await;
      }
    });
  }

  // ──────────────────────────────── Tick ──────────────────────────────────────

  async fn tick(&self) {
    // 1. Clone subscriber entries. Drop lock before any .await into Track/TrackManager.
    let subscriber_snapshot: Vec<(SubscriberKey, SubscriberSnapshot)> = {
      let subs = self.subscribers.read().await;
      subs
        .iter()
        .map(|(key, state)| {
          (
            key.clone(),
            SubscriberSnapshot {
              client: state.client.clone(),
              property_type: state.property_type,
              n: state.n,
              current_selected: state.current_selected.clone(),
              pending_removal_streak: state.pending_removal_streak.clone(),
              is_publisher: state.is_publisher,
            },
          )
        })
        .collect()
    };

    if subscriber_snapshot.is_empty() {
      return;
    }

    let dwell_ticks = self.config.top_n_dwell_ticks;

    // 2. Clone ranker snapshots once (keyed by (prefix, property_type)).
    let ranker_snapshots: HashMap<RankerKey, TopNRanker> = {
      let rankers = self.rankers.read().await;
      rankers.clone()
    };

    // 3. Cache audience top-N results. Audience-only subscribers (is_publisher=false)
    //    with the same (prefix, property_type, n) get identical rankings because they
    //    have no published tracks to self-exclude. Compute once, reuse for all.
    let mut audience_cache: HashMap<(Tuple, u64, usize), HashSet<FullTrackName>> = HashMap::new();

    // 4. For each subscriber, compute diff and execute changes.
    for ((prefix, conn_id), mut snap) in subscriber_snapshot {
      let ranker = match ranker_snapshots.get(&(prefix.clone(), snap.property_type)) {
        Some(r) => r,
        None => continue, // No ranker yet — subscriber registered but no tracks have spoken
      };

      let ranked_set: HashSet<FullTrackName> = if snap.is_publisher {
        // Publisher/panelist: must compute individually due to self-exclusion.
        ranker.compute_top_n(conn_id, snap.n).into_iter().collect()
      } else {
        // Audience-only: reuse cached result for this (prefix, property_type, n).
        let cache_key = (prefix.clone(), snap.property_type, snap.n);
        audience_cache
          .entry(cache_key)
          .or_insert_with(|| {
            // Use usize::MAX as sentinel — no publisher tracks to exclude.
            ranker.compute_top_n(usize::MAX, snap.n).into_iter().collect()
          })
          .clone()
      };

      // Hysteresis: tracks that enter top-N are added immediately.
      // Tracks that leave top-N must stay out for dwell_ticks consecutive ticks before removal.
      let to_add: Vec<FullTrackName> = ranked_set
        .iter()
        .filter(|ftn| !snap.current_selected.contains(*ftn))
        .cloned()
        .collect();

      // Increment removal streak for tracks that fell out; reset for tracks that are still in.
      for ftn in snap.current_selected.iter() {
        if ranked_set.contains(ftn) {
          snap.pending_removal_streak.remove(ftn);
        } else {
          *snap.pending_removal_streak.entry(ftn.clone()).or_insert(0) += 1;
        }
      }

      let to_remove: Vec<FullTrackName> = snap
        .pending_removal_streak
        .iter()
        .filter(|(_, streak)| **streak >= dwell_ticks)
        .map(|(ftn, _)| ftn.clone())
        .collect();

      if to_add.is_empty() && to_remove.is_empty() {
        // Persist updated removal streaks even when no action is taken.
        self
          .update_state(&prefix, conn_id, None, snap.pending_removal_streak.clone())
          .await;
        continue;
      }

      debug!(
        "TopNCoordinator tick: conn={} prefix={:?} to_add={} to_remove={}",
        conn_id,
        prefix,
        to_add.len(),
        to_remove.len()
      );

      // Remove subscriptions for tracks that have been out of top-N long enough.
      for track_name in &to_remove {
        self
          .remove_subscription_for(conn_id, track_name, &mut snap)
          .await;
      }

      // Add subscriptions for newly ranked-in tracks.
      let client_clone = snap.client.clone();
      for track_name in &to_add {
        self
          .add_subscription_for(conn_id, &client_clone, track_name, &mut snap)
          .await;
      }

      // Persist updated state.
      self
        .update_state(
          &prefix,
          conn_id,
          Some(snap.current_selected.clone()),
          snap.pending_removal_streak.clone(),
        )
        .await;
    }
  }

  async fn remove_subscription_for(
    &self,
    conn_id: usize,
    track_name: &FullTrackName,
    snap: &mut SubscriberSnapshot,
  ) {
    if let Some(track_arc) = self.track_manager.get_track(track_name).await {
      let track = track_arc.read().await;
      if let Some(sub_arc) = track.get_subscription(conn_id).await {
        let sub = sub_arc.read().await;
        if let Err(e) = sub
          .send_publish_done(PublishDoneStatusCode::TrackEnded, "Top-N ranked out")
          .await
        {
          warn!(
            "TopNCoordinator: send_publish_done failed for conn={} track={}: {:?}",
            conn_id, track_name, e
          );
        }
      }
      track.remove_subscription(conn_id).await;
    }
    snap.current_selected.remove(track_name);
    snap.pending_removal_streak.remove(track_name);
    info!(
      "TopNCoordinator: removed subscription conn={} track={}",
      conn_id, track_name
    );
  }

  async fn add_subscription_for(
    &self,
    conn_id: usize,
    client: &Arc<MOQTClient>,
    track_name: &FullTrackName,
    snap: &mut SubscriberSnapshot,
  ) {
    let track_arc = match self.track_manager.get_track(track_name).await {
      Some(t) => t,
      None => {
        warn!(
          "TopNCoordinator: track not found for to_add: {}",
          track_name
        );
        return;
      }
    };

    let (_, mut pub_msg) = match self.track_manager.get_any_publish_message(track_name).await {
      Some(p) => p,
      None => {
        warn!(
          "TopNCoordinator: no publish message for track {}",
          track_name
        );
        return;
      }
    };

    let relay_pub_id = self.relay_next_request_id.fetch_add(2, Ordering::Relaxed);

    let relay_track_id = track_arc.read().await.relay_track_id;
    pub_msg.request_id = relay_pub_id;
    pub_msg.track_alias = relay_track_id;

    {
      let mut pending = self.relay_pending_requests.write().await;
      pending.insert(
        relay_pub_id,
        PendingRequest::Publish {
          publisher_connection_id: conn_id,
          original_request_id: relay_pub_id,
          message: pub_msg.clone(),
        },
      );
    }

    client
      .queue_message(ControlMessage::Publish(Box::new(pub_msg.clone())))
      .await;

    let track = track_arc.read().await;
    if let Err(e) = track.add_subscription(client.clone(), pub_msg, false).await {
      warn!(
        "TopNCoordinator: add_subscription failed for conn={} track={}: {:?}",
        conn_id, track_name, e
      );
    } else {
      snap.current_selected.insert(track_name.clone());
      info!(
        "TopNCoordinator: added subscription conn={} track={}",
        conn_id, track_name
      );
    }
  }

  async fn update_state(
    &self,
    prefix: &Tuple,
    conn_id: usize,
    new_selected: Option<HashSet<FullTrackName>>,
    new_streak: HashMap<FullTrackName, u32>,
  ) {
    let mut subs = self.subscribers.write().await;
    if let Some(state) = subs.get_mut(&(prefix.clone(), conn_id)) {
      if let Some(sel) = new_selected {
        state.current_selected = sel;
      }
      state.pending_removal_streak = new_streak;
    }
  }

  /// Remove rankers that have no subscribers (prevents unbounded memory growth).
  async fn gc_empty_rankers(&self) {
    let subs = self.subscribers.read().await;
    let active_keys: HashSet<(Tuple, u64)> = subs
      .iter()
      .map(|((prefix, _), state)| (prefix.clone(), state.property_type))
      .collect();
    drop(subs);

    let mut rankers = self.rankers.write().await;
    rankers.retain(|key, _| active_keys.contains(key));
  }
}

// ─────────────────────────── Snapshot helper ────────────────────────────────

/// Owned snapshot of a subscriber's state for use in the tick loop
/// (avoids holding the subscribers lock while doing async I/O).
struct SubscriberSnapshot {
  client: Arc<MOQTClient>,
  property_type: u64,
  n: usize,
  current_selected: HashSet<FullTrackName>,
  pending_removal_streak: HashMap<FullTrackName, u32>,
  is_publisher: bool,
}

// ─────────────────────────── Extension helpers ──────────────────────────────

/// Scan object extensions for a VarInt-encoded KVP with the given type key.
/// Checks both top-level Unknown KVPs and inner KVPs inside ImmutableExtensions.
fn extract_varint_extension(extensions: &[ObjectExtension], key: u64) -> Option<u64> {
  for ext in extensions {
    match ext {
      ObjectExtension::Unknown { kvp } => {
        if let KeyValuePair::VarInt { type_value, value } = kvp
          && *type_value == key
        {
          return Some(*value);
        }
      }
      ObjectExtension::ImmutableExtensions { extensions: inner } => {
        for kvp in inner {
          if let KeyValuePair::VarInt { type_value, value } = kvp
            && *type_value == key
          {
            return Some(*value);
          }
        }
      }
      _ => {}
    }
  }
  None
}
