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
use crate::server::utils;
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::object::Object;
use moqtail::{model::common::tuple::Tuple, transport::data_stream_handler::HeaderInfo};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedSender;
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
  subscriptions: Arc<RwLock<BTreeMap<usize, Arc<RwLock<Subscription>>>>>,
  pub publisher_connection_id: usize,
  #[allow(dead_code)]
  pub(crate) cache: TrackCache,
  subscriber_senders: Arc<RwLock<BTreeMap<usize, UnboundedSender<TrackEvent>>>>,
  pub largest_location: Arc<RwLock<Location>>,
  pub object_logger: ObjectLogger,
  log_folder: String,
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
      subscriptions: Arc::new(RwLock::new(BTreeMap::new())),
      publisher_connection_id,
      cache: TrackCache::new(track_alias, config.cache_size.into(), config),
      subscriber_senders: Arc::new(RwLock::new(BTreeMap::new())),
      largest_location: Arc::new(RwLock::new(Location::new(0, 0))),
      object_logger: ObjectLogger::new(config.log_folder.clone()),
      log_folder: config.log_folder.clone(),
      config,
    }
  }

  pub async fn add_subscription(
    &mut self,
    subscriber: Arc<MOQTClient>,
    subscribe_message: Subscribe,
  ) -> Result<(), anyhow::Error> {
    let connection_id = { subscriber.connection_id };

    info!(
      "Adding subscription for subscriber_id: {} to track: {}",
      connection_id, self.track_alias
    );

    // Create a separate unbounded channel for this subscriber
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<TrackEvent>();

    let subscription = Subscription::new(
      self.track_alias,
      subscribe_message,
      subscriber.clone(),
      event_rx,
      self.cache.clone(),
      connection_id,
      self.log_folder.clone(),
      self.config,
    );

    let mut subscriptions = self.subscriptions.write().await;
    if subscriptions.contains_key(&connection_id) {
      error!(
        "Subscriber with connection_id: {} already exists in track: {}",
        connection_id, self.track_alias
      );
      return Err(anyhow::anyhow!("Subscriber already exists"));
    }
    subscriptions.insert(connection_id, Arc::new(RwLock::new(subscription)));

    // Store the sender for this subscriber
    let mut senders = self.subscriber_senders.write().await;
    senders.insert(connection_id, event_tx);

    Ok(())
  }

  pub async fn remove_subscription(&mut self, subscriber_id: usize) {
    info!(
      "Removing subscription for subscriber_id: {} from track: {}",
      subscriber_id, self.track_alias
    );
    let mut subscriptions = self.subscriptions.write().await;
    // find the subscription by subscriber_id and finish it
    if let Some(subscription) = subscriptions.get(&subscriber_id) {
      let mut sub = subscription.write().await;
      sub.finish().await;
    }
    subscriptions.remove(&subscriber_id);

    // Remove and dispose the sender for this subscriber
    let mut senders = self.subscriber_senders.write().await;
    if let Some(_sender) = senders.remove(&subscriber_id) {
      // The sender is automatically dropped here, which closes the channel
      info!(
        "Disposed sender for subscriber_id: {} from track: {}",
        subscriber_id, self.track_alias
      );
    }
  }

  pub async fn new_object(
    &self,
    stream_id: &StreamId,
    object: &Object,
  ) -> Result<(), anyhow::Error> {
    self.new_object_with_header(stream_id, object, None).await
  }

  pub async fn new_object_with_header(
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

      self.send_event_to_subscribers(event).await?;
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

    self.send_event_to_subscribers(event).await?;

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

    self.send_event_to_subscribers(event).await?;

    Ok(())
  }

  // Send event to all subscribers
  async fn send_event_to_subscribers(
    &self,
    event: TrackEvent,
  ) -> Result<Vec<usize>, anyhow::Error> {
    let senders = self.subscriber_senders.read().await;
    let mut failed_subscribers = Vec::new();

    if !senders.is_empty() {
      for (subscriber_id, sender) in senders.iter() {
        if let Err(e) = sender.send(event.clone()) {
          error!(
            "Failed to send event to subscriber {}: {}",
            subscriber_id, e
          );
          failed_subscribers.push(*subscriber_id);
        }
      }

      if !failed_subscribers.is_empty() {
        error!(
          "{:?} event sent to {} subscribers, {} failed for track: {}",
          event,
          senders.len() - failed_subscribers.len(),
          failed_subscribers.len(),
          self.track_alias
        );
      } else {
        debug!(
          "{:?} event sent successfully to {} subscribers for track: {}",
          event,
          senders.len(),
          self.track_alias
        );
      }
    }

    Ok(failed_subscribers)
  }
}

// TODO: Test
