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

use super::track_cache::TrackCache;
use crate::server::client::MOQTClient;
use crate::server::config::AppConfig;
use crate::server::object_logger::ObjectLogger;
use crate::server::stream_id::StreamId;
use crate::server::subscription::Subscription;
use crate::server::subscription_manager::SubscriptionManager;
use crate::server::utils;
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::SubscribeErrorCode;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::constant::ObjectForwardingPreference;
use moqtail::model::data::datagram_object::DatagramObject;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::data::object::Object;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};
use tracing::{debug, error, info, warn};

/// Lifecycle status of a track on the relay.
#[derive(Debug, Clone)]
pub enum TrackStatus {
  /// Track created, subscribe forwarded to publisher, awaiting response.
  Pending,
  /// Publisher confirmed with SubscribeOk.
  Confirmed {
    publisher_track_alias: u64,
    expires: u64,
    largest_location: Option<Location>,
  },
  /// Publisher rejected with SubscribeError.
  Rejected {
    error_code: SubscribeErrorCode,
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
  DatagramObject {
    object: DatagramObject,
  },
  StreamClosed {
    stream_id: StreamId,
  },
  PublisherDisconnected {
    reason: String,
  },
}
#[derive(Debug, Clone)]
pub struct Track {
  pub track_alias: u64,
  pub full_track_name: FullTrackName,
  pub subscription_manager: SubscriptionManager,
  pub publisher_connection_id: usize,
  pub(crate) cache: TrackCache,
  pub largest_location: Arc<RwLock<Location>>,
  pub object_logger: ObjectLogger,
  config: &'static AppConfig,
  pub forwarding_preference: Arc<RwLock<ObjectForwardingPreference>>,
  pub status: Arc<RwLock<TrackStatus>>,
  pub status_notify: Arc<Notify>,
  /// Subscribers waiting for track confirmation: (request_id, connection_id).
  pub pending_subscribers: Arc<RwLock<Vec<(u64, usize)>>>,
}

// TODO: this track implementation should be static? At least
// its lifetime should be same as the server's lifetime
impl Track {
  pub fn new(
    track_alias: u64,
    full_track_name: FullTrackName,
    publisher_connection_id: usize,
    config: &'static AppConfig,
    initial_status: TrackStatus,
  ) -> Self {
    Track {
      track_alias,
      full_track_name: full_track_name.clone(),
      subscription_manager: SubscriptionManager::new(
        track_alias,
        full_track_name,
        config.log_folder.clone(),
        config,
      ),
      publisher_connection_id,
      cache: TrackCache::new(track_alias, config.cache_size.into(), config),
      largest_location: Arc::new(RwLock::new(Location::new(0, 0))),
      object_logger: ObjectLogger::new(config.log_folder.clone()),
      config,
      forwarding_preference: Arc::new(RwLock::new(ObjectForwardingPreference::Subgroup)),
      status: Arc::new(RwLock::new(initial_status)),
      status_notify: Arc::new(Notify::new()),
      pending_subscribers: Arc::new(RwLock::new(Vec::new())),
    }
  }

  /// Transition from Pending to Confirmed. Updates track_alias and notifies waiters.
  pub async fn confirm(
    &mut self,
    publisher_track_alias: u64,
    expires: u64,
    largest_location: Option<Location>,
  ) {
    self.track_alias = publisher_track_alias;
    self
      .subscription_manager
      .update_track_alias(publisher_track_alias);
    let mut status = self.status.write().await;
    *status = TrackStatus::Confirmed {
      publisher_track_alias,
      expires,
      largest_location,
    };
    drop(status);
    self.status_notify.notify_waiters();
  }

  /// Transition from Pending to Rejected. Notifies waiters.
  pub async fn reject(&self, error_code: SubscribeErrorCode, reason_phrase: ReasonPhrase) {
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

  pub async fn set_forwarding_preference(&self, preference: ObjectForwardingPreference) {
    *self.forwarding_preference.write().await = preference;
  }

  pub async fn add_subscription(
    &self,
    subscriber: Arc<MOQTClient>,
    subscribe_message: Subscribe,
    is_switch: bool,
  ) -> Result<Arc<RwLock<Subscription>>, anyhow::Error> {
    // Check if subscription already exists

    if let Some(sub_guard) = self
      .subscription_manager
      .get_subscription(subscriber.connection_id)
      .await
    {
      if !is_switch {
        error!(
          "Subscriber with connection_id: {} already exists in track: {}",
          subscriber.connection_id, self.track_alias
        );
      } else {
        info!(
          "Subscriber with connection_id: {} already exists in track: {} (switch subscription)",
          subscriber.connection_id, self.track_alias
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
      .add_subscription(subscriber, subscribe_message, self.cache.clone())
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

  pub async fn new_subgroup_object(
    &self,
    stream_id: &StreamId,
    object: &Object,
    header_info: Option<&HeaderInfo>,
  ) -> Result<(), anyhow::Error> {
    debug!(
      "new_subgroup_object: track: {:?} location: {:?} stream_id: {} diff_ms: {}",
      object.track_alias,
      object.location,
      stream_id,
      utils::passed_time_since_start()
    );

    if header_info.is_some() {
      info!(
        "new group: track: {:?} location: {:?} stream_id: {} time: {}",
        object.track_alias,
        object.location,
        stream_id,
        utils::passed_time_since_start()
      );
    }

    if let Ok(fetch_object) = object.clone().try_into_fetch() {
      self.cache.add_object(fetch_object).await;
    } else {
      warn!(
        "new_subgroup_object: object cannot be cached | track: {:?} location: {:?} stream_id: {} diff_ms: {} object: {:?}",
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
        .log_track_object(self.track_alias, object, object_received_time)
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
    Ok(())
  }

  pub async fn new_datagram_object(
    &self,
    datagram_object: &DatagramObject,
  ) -> Result<(), anyhow::Error> {
    debug!(
      "new_datagram_object: track: {:?} group: {:?} object_id: {} diff_ms: {}",
      datagram_object.track_alias,
      datagram_object.group_id,
      datagram_object.object_id,
      utils::passed_time_since_start()
    );

    match Object::try_from_datagram(datagram_object.clone()) {
      Ok(object) => {
        if let Ok(fetch_object) = object.clone().try_into_fetch() {
          self.cache.add_object(fetch_object).await;
        } else {
          warn!(
            "new_datagram_object: object cannot be cached | track: {:?} group: {:?} object_id: {} diff_ms: {} object: {:?}",
            datagram_object.track_alias,
            datagram_object.group_id,
            datagram_object.object_id,
            utils::passed_time_since_start(),
            object
          );
        }

        // Track-level logging - log every object arrival if enabled
        if self.config.enable_object_logging {
          let object_received_time = utils::passed_time_since_start();

          self
            .object_logger
            .log_track_object(self.track_alias, &object, object_received_time)
            .await;
        }
      }
      Err(e) => {
        error!(
          "Failed to convert datagram object to object for logging: group: {:?} object_id: {} error: {}",
          datagram_object.group_id, datagram_object.object_id, e
        );
      }
    }

    // update the largest location
    {
      let mut largest_location = self.largest_location.write().await;
      if datagram_object.group_id > largest_location.group
        || (datagram_object.group_id == largest_location.group
          && datagram_object.object_id > largest_location.object)
      {
        largest_location.group = datagram_object.group_id;
        largest_location.object = datagram_object.object_id;
      }
    }

    let event = TrackEvent::DatagramObject {
      object: datagram_object.clone(),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }

  pub async fn stream_closed(&self, stream_id: &StreamId) -> Result<(), anyhow::Error> {
    let event = TrackEvent::StreamClosed {
      stream_id: stream_id.clone(),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }

  /// Send PublisherDisconnected event to all subscribers
  pub async fn notify_publisher_disconnected(&self) -> Result<(), anyhow::Error> {
    info!(
      "Publisher disconnected for track: {} - notifying all subscribers",
      self.track_alias
    );

    let event = TrackEvent::PublisherDisconnected {
      reason: "Publisher disconnected".to_string(),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }
}

// TODO: Test
