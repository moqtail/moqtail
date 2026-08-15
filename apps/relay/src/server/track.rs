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

use super::seen_objects::SeenObjects;
use super::track_cache::TrackCache;
use crate::server::config::AppConfig;
use crate::server::object_logger::ObjectLogger;
use crate::server::stream_id::StreamId;
use crate::server::subscription::Subscription;
use crate::server::subscription_manager::SubscriptionManager;
use crate::server::utils;
use crate::server::{client::MOQTClient, subscription::SubscriptionOrigin};
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::data::datagram::Datagram;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::data::object::Object;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::model::property::track_property::TrackProperty;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Mutex, Notify, RwLock, oneshot};
use tracing::{debug, error, info, warn};

pub type ActiveSubgroupHeaderMap = Arc<RwLock<HashMap<StreamId, HeaderInfo>>>;

/// What created a track. Decides whether it survives its last subscriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackOrigin {
  /// Created by a downstream SUBSCRIBE. The relay owns the upstream
  /// subscription; with no subscribers left it is cancelled and the track goes
  /// stale, so it must be removed to force a fresh upstream SUBSCRIBE.
  Subscribe,
  /// Created by an upstream PUBLISH. The publisher keeps pushing regardless of
  /// subscriber count, so the track outlives them.
  Publish,
}

/// Lifecycle status of a track on the relay.
#[derive(Debug, Clone)]
pub enum TrackStatus {
  /// Track created, subscribe forwarded to publisher, awaiting response.
  Pending,
  /// Publisher confirmed with SubscribeOk, with the parameters it sent. They were
  /// addressed to the relay, so they are what a later SUBSCRIBE_OK is derived from,
  /// never what it forwards.
  Confirmed {
    upstream_parameters: Vec<MessageParameter>,
  },
  /// Publisher rejected with RequestError.
  Rejected {
    error_code: RequestErrorCode,
    reason_phrase: ReasonPhrase,
  },
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum TrackEvent {
  SubgroupObject {
    stream_id: StreamId,
    object: Object,
    header_info: Option<HeaderInfo>,
  },
  Datagram {
    object: Datagram,
  },
  StreamClosed {
    stream_id: StreamId,
  },
  PublisherDisconnected {
    status_code: PublishDoneStatusCode,
    reason: String,
  },
}

#[derive(Debug, Clone)]
pub struct Track {
  /// Stable relay-assigned track identifier, independent of publisher aliases.
  pub relay_track_id: u64,
  pub full_track_name: FullTrackName,
  pub origin: TrackOrigin,
  pub subscription_manager: SubscriptionManager,
  /// Maps publisher_connection_id -> publisher_track_alias for all active publishers.
  pub publisher_aliases: Arc<RwLock<BTreeMap<usize, u64>>>,
  pub(crate) cache: TrackCache,
  pub largest_location: Arc<RwLock<Location>>,
  /// Object ids already ingested, to drop duplicates from multiple publishers.
  seen_objects: Arc<SeenObjects>,
  /// Set once at least one Object (subgroup or datagram) has been seen for this
  /// track, so `largest_location` becomes meaningful (it starts at {0,0}).
  has_objects: Arc<AtomicBool>,
  pub object_logger: ObjectLogger,
  config: &'static AppConfig,
  pub status: Arc<RwLock<TrackStatus>>,
  pub status_notify: Arc<Notify>,
  /// Subscribers waiting for track confirmation: (request_id, connection_id).
  pub pending_subscribers: Arc<RwLock<Vec<(u64, usize)>>>,
  /// Cached track properties from PUBLISH or SUBSCRIBE_OK (relays MUST cache).
  pub track_properties: Arc<RwLock<Vec<TrackProperty>>>,
  /// Original subgroup headers for open publisher streams, keyed by stream_id.
  /// Used so new mid-group subscribers can open a QUIC send stream
  /// Inserted when the first object of a subgroup arrives; removed when the
  /// publisher's unistream closes (stream_closed signal).
  pub active_subgroup_headers: ActiveSubgroupHeaderMap,
  /// One per publisher; firing it resets that relay-owned upstream SUBSCRIBE stream
  /// when the last downstream subscriber goes away. Only the pull path fills this, so
  /// a PUBLISH-created track leaves it empty and a subscriber leaving never cancels
  /// the publisher.
  pub upstream_subscribe_cancellers: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
  /// Upstream SUBSCRIBEs still awaiting a SUBSCRIBE_OK or REQUEST_ERROR.
  pub pending_upstream_subscribe_count: Arc<AtomicUsize>,
  /// Forward State the relay last gave the upstream publisher of a
  /// PUBLISH-created track. A relay may answer PUBLISH with Forward State 0
  /// while nothing downstream wants the track; this records whether it has
  /// since been raised, so it is raised only once.
  pub upstream_forward: Arc<AtomicBool>,
}

// TODO: this track implementation should be static? At least
// its lifetime should be same as the server's lifetime
impl Track {
  /// SSTS: remove this track from the switching sets of all its subscribers
  /// (decrementing `activate`, deleting emptied sets).
  pub async fn remove_from_subscriber_switching_sets(&self) {
    let subscriptions = self.subscription_manager.get_all_subscriptions().await;
    for sub in &subscriptions {
      let subscriber = sub.read().await.subscriber().clone();
      subscriber
        .switching_sets
        .write()
        .await
        .remove(&self.full_track_name);
    }
  }

  pub fn new(
    relay_track_id: u64,
    full_track_name: FullTrackName,
    config: &'static AppConfig,
    initial_status: TrackStatus,
    origin: TrackOrigin,
  ) -> Self {
    Track {
      relay_track_id,
      full_track_name: full_track_name.clone(),
      origin,
      subscription_manager: SubscriptionManager::new(
        relay_track_id,
        full_track_name,
        config.log_folder.clone(),
        config,
      ),
      publisher_aliases: Arc::new(RwLock::new(BTreeMap::new())),
      cache: TrackCache::new(relay_track_id, config.cache_size.into(), config),
      largest_location: Arc::new(RwLock::new(Location::new(0, 0))),
      seen_objects: Arc::new(SeenObjects::new(config.dedup_retained_groups)),
      has_objects: Arc::new(AtomicBool::new(false)),
      object_logger: ObjectLogger::new(config.log_folder.clone()),
      config,
      status: Arc::new(RwLock::new(initial_status)),
      status_notify: Arc::new(Notify::new()),
      pending_subscribers: Arc::new(RwLock::new(Vec::new())),
      track_properties: Arc::new(RwLock::new(Vec::new())),
      active_subgroup_headers: Arc::new(RwLock::new(HashMap::new())),
      upstream_subscribe_cancellers: Arc::new(Mutex::new(Vec::new())),
      pending_upstream_subscribe_count: Arc::new(AtomicUsize::new(0)),
      upstream_forward: Arc::new(AtomicBool::new(false)),
    }
  }

  /// Whether any subscription on this track currently wants Objects forwarded.
  pub async fn has_forwarding_subscriber(&self) -> bool {
    for sub in self.subscription_manager.get_all_subscriptions().await {
      if sub.read().await.is_forwarding().await {
        return true;
      }
    }
    false
  }

  /// Add a publisher (connection_id -> track_alias) to this track.
  pub async fn add_publisher(&self, connection_id: usize, track_alias: u64) {
    let mut aliases = self.publisher_aliases.write().await;
    aliases.insert(connection_id, track_alias);
    info!(
      "Added publisher {}@alias={} to relay_track_id={}",
      connection_id, track_alias, self.relay_track_id
    );
  }

  /// Remove a publisher by connection_id. Returns the removed alias if found.
  /// If no publishers remain after removal, sends PublisherDisconnected to all subscribers.
  pub async fn remove_publisher(&self, connection_id: usize) -> Option<u64> {
    let removed_alias = {
      let mut aliases = self.publisher_aliases.write().await;
      aliases.remove(&connection_id)
    };

    if let Some(alias) = removed_alias {
      let has_publishers = !self.publisher_aliases.read().await.is_empty();
      info!(
        "Removed publisher {}@alias={} from relay_track_id={} | publishers_remaining={}",
        connection_id, alias, self.relay_track_id, has_publishers
      );

      if !has_publishers && let Err(e) = self.notify_publisher_disconnected().await {
        error!(
          "Failed to notify subscribers after last publisher removed for relay_track_id={}: {:?}",
          self.relay_track_id, e
        );
      }
    }

    removed_alias
  }

  /// Returns true if there is at least one active publisher for this track.
  pub async fn has_publishers(&self) -> bool {
    !self.publisher_aliases.read().await.is_empty()
  }

  /// Whether the given connection is one of this track's publishers.
  pub async fn is_published_by(&self, connection_id: usize) -> bool {
    self
      .publisher_aliases
      .read()
      .await
      .contains_key(&connection_id)
  }

  /// The Largest Object seen for this track, or None if no Objects have been
  /// published yet (so `largest_location`'s default {0,0} is not mistaken for
  /// a real Object).
  pub async fn largest_object(&self) -> Option<Location> {
    if self.has_objects.load(Ordering::Relaxed) {
      Some(self.largest_location.read().await.clone())
    } else {
      None
    }
  }

  /// Records an Object's location and reports whether it had already been seen.
  /// Only the Object's own group is locked, so this does not serialise the track.
  fn is_duplicate(&self, location: &Location) -> bool {
    self.seen_objects.is_duplicate(location)
  }

  /// Registers an accepting publisher. Returns true only for the one that took the
  /// track out of Pending, since only it has a downstream response to send.
  pub async fn confirm(
    &mut self,
    publisher_connection_id: usize,
    publisher_track_alias: u64,
    upstream_parameters: Vec<MessageParameter>,
    properties: Vec<TrackProperty>,
  ) -> bool {
    {
      let mut aliases = self.publisher_aliases.write().await;
      aliases.insert(publisher_connection_id, publisher_track_alias);
    }
    *self.track_properties.write().await = properties;

    let mut status = self.status.write().await;
    let was_pending = matches!(*status, TrackStatus::Pending);
    if was_pending {
      *status = TrackStatus::Confirmed {
        upstream_parameters,
      };
    }
    drop(status);

    if was_pending {
      self.status_notify.notify_waiters();
    }

    info!(
      "Track publisher accepted: relay_track_id={} publisher_connection_id={} publisher_alias={} confirmed_now={}",
      self.relay_track_id, publisher_connection_id, publisher_track_alias, was_pending
    );
    was_pending
  }

  /// Updates the cached track properties (per spec: most recent set replaces any previous).
  pub async fn set_track_properties(&self, properties: Vec<TrackProperty>) {
    *self.track_properties.write().await = properties;
  }

  /// Transition from Pending to Rejected. Notifies waiters.
  pub async fn reject(&self, error_code: RequestErrorCode, reason_phrase: ReasonPhrase) {
    let mut status = self.status.write().await;
    *status = TrackStatus::Rejected {
      error_code,
      reason_phrase,
    };
    drop(status);
    self.status_notify.notify_waiters();
  }

  pub async fn get_status(&self) -> TrackStatus {
    self.status.read().await.clone()
  }

  pub async fn add_subscription(
    &self,
    subscriber: Arc<MOQTClient>,
    origin_message: impl Into<SubscriptionOrigin>,
    is_switch: bool,
  ) -> Result<Arc<RwLock<Subscription>>, anyhow::Error> {
    let origin_enum = origin_message.into();
    // Check if subscription already exists
    if let Some(sub_guard) = self
      .subscription_manager
      .get_subscription(subscriber.connection_id)
      .await
    {
      if !is_switch {
        error!(
          "Subscriber with connection_id: {} already exists in relay_track_id={}",
          subscriber.connection_id, self.relay_track_id
        );
      } else {
        info!(
          "Subscriber with connection_id: {} already exists in relay_track_id={} (switch subscription)",
          subscriber.connection_id, self.relay_track_id
        );
        // inform the existing subscription about the switch
        let sub = sub_guard.read().await;
        sub.notify_switch().await;
      }
      return Err(anyhow::anyhow!(
        "A subscription already exists for this subscriber"
      ));
    }

    let subscription = self
      .subscription_manager
      .add_subscription(
        subscriber,
        origin_enum,
        self.cache.clone(),
        Arc::clone(&self.active_subgroup_headers),
      )
      .await?;

    if is_switch {
      subscription.read().await.notify_switch().await;
    }

    Ok(subscription)
  }

  // return the subscription for the client
  // subscriber_id is the connection id of the client
  pub async fn get_subscription(&self, subscriber_id: usize) -> Option<Arc<RwLock<Subscription>>> {
    self
      .subscription_manager
      .get_subscription(subscriber_id)
      .await
  }

  pub async fn remove_subscription(&self, subscriber_id: usize) {
    self
      .subscription_manager
      .remove_subscription(subscriber_id)
      .await
  }

  pub async fn subscriber_count(&self) -> usize {
    self.subscription_manager.subscriber_count().await
  }

  pub async fn new_subgroup_object(
    &self,
    stream_id: &StreamId,
    object: &Object,
    header_info: Option<&HeaderInfo>,
  ) -> Result<(), anyhow::Error> {
    debug!(
      "new_subgroup_object: relay_track_id={} location: {:?} stream_id={} diff_ms={}",
      self.relay_track_id,
      object.location,
      stream_id,
      utils::passed_time_since_start()
    );

    if self.is_duplicate(&object.location) {
      debug!(
        "new_subgroup_object: dropping duplicate | relay_track_id={} location: {:?}",
        self.relay_track_id, object.location
      );
      return Ok(());
    }

    if let Some(h) = header_info {
      info!(
        "new group: relay_track_id={} location: {:?} stream_id={} time={}",
        self.relay_track_id,
        object.location,
        stream_id,
        utils::passed_time_since_start()
      );
      self
        .active_subgroup_headers
        .write()
        .await
        .insert(stream_id.clone(), h.clone());
    }

    // Send single Object event with optional header info
    let event = TrackEvent::SubgroupObject {
      stream_id: stream_id.clone(),
      object: object.clone(),
      header_info: header_info.cloned(),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    if let Ok(fetch_object) = object.clone().try_into_fetch() {
      self.cache.add_object(fetch_object).await;
    } else {
      warn!(
        "new_subgroup_object: object cannot be cached | relay_track_id: {} track_alias: {} location: {:?} stream_id: {} diff_ms: {} object: {:?}",
        self.relay_track_id,
        object.track_alias,
        object.location,
        stream_id,
        utils::passed_time_since_start(),
        object
      );
    }

    // Track-level logging - log every object arrival if enabled
    if self.config.enable_object_logging {
      let object_received_time = utils::passed_time_since_start();
      self
        .object_logger
        .log_track_object(self.relay_track_id, object, object_received_time)
        .await;
    }

    // update the largest location
    {
      let mut largest_location = self.largest_location.write().await;
      if object.location.group > largest_location.group
        || (object.location.group == largest_location.group
          && object.location.object > largest_location.object)
      {
        largest_location.group = object.location.group;
        largest_location.object = object.location.object;
      }
    }
    self.has_objects.store(true, Ordering::Relaxed);
    Ok(())
  }

  pub async fn new_datagram(&self, datagram: &Datagram) -> Result<(), anyhow::Error> {
    debug!(
      "new_datagram: relay_track_id={} group: {:?} object_id={} diff_ms={}",
      self.relay_track_id,
      datagram.group_id,
      datagram.object_id,
      utils::passed_time_since_start()
    );

    match Object::try_from_datagram(datagram.clone(), 0) {
      Ok((object, end_of_group)) => {
        if self.is_duplicate(&object.location) {
          debug!(
            "new_datagram: dropping duplicate | relay_track_id={} location: {:?}",
            self.relay_track_id, object.location
          );
          return Ok(());
        }

        if end_of_group {
          debug!(
            "new_datagram: end_of_group received for track: {:?} group: {:?} object_id: {}",
            datagram.track_alias, datagram.group_id, datagram.object_id
          );
        }

        if let Ok(fetch_object) = object.clone().try_into_fetch() {
          self.cache.add_object(fetch_object).await;
        } else {
          warn!(
            "new_datagram: object cannot be cached | relay_track_id={} group: {:?} object_id={} diff_ms={} object: {:?}",
            self.relay_track_id,
            datagram.group_id,
            datagram.object_id,
            utils::passed_time_since_start(),
            object
          );
        }

        // Track-level logging - log every object arrival if enabled
        if self.config.enable_object_logging {
          let object_received_time = utils::passed_time_since_start();
          self
            .object_logger
            .log_track_object(self.relay_track_id, &object, object_received_time)
            .await;
        }
      }
      Err(e) => {
        error!(
          "Failed to convert datagram to object for logging: group: {:?} object_id={} error={}",
          datagram.group_id, datagram.object_id, e
        );
      }
    }

    // update the largest location
    {
      let mut largest_location = self.largest_location.write().await;
      if datagram.group_id > largest_location.group
        || (datagram.group_id == largest_location.group
          && datagram.object_id > largest_location.object)
      {
        largest_location.group = datagram.group_id;
        largest_location.object = datagram.object_id;
      }
    }
    self.has_objects.store(true, Ordering::Relaxed);

    let event = TrackEvent::Datagram {
      object: datagram.clone(),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }

  pub async fn stream_closed(&self, stream_id: &StreamId) -> Result<(), anyhow::Error> {
    self.active_subgroup_headers.write().await.remove(stream_id);

    let event = TrackEvent::StreamClosed {
      stream_id: stream_id.clone(),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }

  /// Send PublisherDisconnected event to all subscribers.
  /// Called internally by remove_publisher() when the last publisher leaves.
  pub async fn notify_publisher_disconnected(&self) -> Result<(), anyhow::Error> {
    info!(
      "All publishers gone for relay_track_id={} - notifying all subscribers",
      self.relay_track_id
    );

    self
      .notify_publish_done(
        PublishDoneStatusCode::TrackEnded,
        "Publisher disconnected".to_string(),
      )
      .await
  }

  /// Fan a PUBLISH_DONE out to all subscribers with the given status/reason.
  /// Used when an upstream publisher signals it is done for the track, so the
  /// upstream's status code is relayed downstream verbatim rather than being
  /// flattened to a generic disconnect.
  pub async fn notify_publish_done(
    &self,
    status_code: PublishDoneStatusCode,
    reason: String,
  ) -> Result<(), anyhow::Error> {
    let event = TrackEvent::PublisherDisconnected {
      status_code,
      reason,
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }
}
