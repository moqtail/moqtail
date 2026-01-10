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
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::object::Object;
use moqtail::{model::common::tuple::Tuple, transport::data_stream_handler::HeaderInfo};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum TrackEvent {
  Object {
    stream_id: StreamId,
    object: Object,
    header_info: Option<HeaderInfo>,
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
  #[allow(dead_code)]
  pub track_alias: u64,
  #[allow(dead_code)]
  pub track_namespace: Tuple,
  #[allow(dead_code)]
  pub track_name: String,
  pub subscription_manager: SubscriptionManager,
  pub publisher_connection_id: usize,
  #[allow(dead_code)]
  pub(crate) cache: TrackCache,
  pub largest_location: Arc<RwLock<Location>>,
  pub object_logger: ObjectLogger,
  config: &'static AppConfig,
}

// TODO: this track implementation should be static? At least
// its lifetime should be same as the server's lifetime
impl Track {
  pub fn new(
    track_alias: u64,
    track_namespace: Tuple,
    track_name: String,
    publisher_connection_id: usize,
    config: &'static AppConfig,
  ) -> Self {
    Track {
      track_alias,
      track_namespace,
      track_name,
      subscription_manager: SubscriptionManager::new(
        track_alias,
        config.log_folder.clone(),
        config,
      ),
      publisher_connection_id,
      cache: TrackCache::new(track_alias, config.cache_size.into(), config),
      largest_location: Arc::new(RwLock::new(Location::new(0, 0))),
      object_logger: ObjectLogger::new(config.log_folder.clone()),
      config,
    }
  }

  pub async fn add_subscription(
    &self,
    subscriber: Arc<MOQTClient>,
    subscribe_message: Subscribe,
  ) -> Result<(), anyhow::Error> {
    self
      .subscription_manager
      .add_subscription(subscriber, subscribe_message, self.cache.clone())
      .await
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

  pub async fn new_object(
    &self,
    stream_id: &StreamId,
    object: &Object,
    header_info: Option<&HeaderInfo>,
  ) -> Result<(), anyhow::Error> {
    debug!(
      "new_object: track: {:?} location: {:?} stream_id: {} diff_ms: {}",
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
      let event = TrackEvent::Object {
        stream_id: stream_id.clone(),
        object: object.clone(),
        header_info: header_info.cloned(),
      };

      self
        .subscription_manager
        .send_event_to_subscribers(event)
        .await?;
      Ok(())
    } else {
      error!(
        "new_object: track: {:?} location: {:?} stream_id: {} diff_ms: {} object: {:?}",
        object.track_alias,
        object.location,
        stream_id,
        utils::passed_time_since_start(),
        object
      );
      Err(anyhow::anyhow!("Object is not a fetch object"))
    }
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
