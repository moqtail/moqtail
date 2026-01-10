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
use super::config::AppConfig;
use super::subscription::Subscription;
use super::track::TrackEvent;
use super::utils;
use anyhow::Result;
use moqtail::model::control::subscribe::Subscribe;
use std::{collections::BTreeMap, collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, error, info};

/// Number of partitions for subscriber senders management to reduce lock contention.
/// Each partition contains a separate HashMap protected by its own RwLock.
/// Higher values reduce contention but increase memory overhead.
/// Should be a power of 2 for optimal modulo performance.
const SUBSCRIBER_PARTITION_COUNT: usize = 16;

pub type SubscriberSenderMap = HashMap<usize, UnboundedSender<TrackEvent>>;
pub type SubscriberSenderLock = Arc<RwLock<SubscriberSenderMap>>;
pub type SubscriberSenderList = Vec<SubscriberSenderLock>;

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionManager {
  track_alias: u64,
  subscriptions: Arc<RwLock<BTreeMap<usize, Arc<RwLock<Subscription>>>>>,
  subscriber_senders: Arc<SubscriberSenderList>,
  log_folder: String,
  config: &'static AppConfig,
}

impl SubscriptionManager {
  pub fn new(track_alias: u64, log_folder: String, config: &'static AppConfig) -> Self {
    // Initialize partitioned subscriber senders
    let mut subscriber_senders = Vec::with_capacity(SUBSCRIBER_PARTITION_COUNT);
    for _ in 0..SUBSCRIBER_PARTITION_COUNT {
      subscriber_senders.push(Arc::new(RwLock::new(HashMap::new())));
    }

    SubscriptionManager {
      track_alias,
      subscriptions: Arc::new(RwLock::new(BTreeMap::new())),
      subscriber_senders: Arc::from(subscriber_senders),
      log_folder,
      config,
    }
  }

  /// Calculate the partition index for subscriber distribution across buckets.
  /// This method implements a load balancing strategy to distribute subscribers
  /// across multiple buckets to improve performance and reduce contention.
  fn get_subscriber_partition_index(&self, subscriber_id: usize) -> usize {
    // Convert to bytes for fnv_hash function
    let value_bytes = subscriber_id.to_le_bytes();
    (utils::fnv_hash(&value_bytes) % SUBSCRIBER_PARTITION_COUNT as u64) as usize
  }

  /// Get the subscriber sender for a specific subscriber
  async fn _get_subscriber_sender(
    &self,
    subscriber_id: usize,
  ) -> Option<UnboundedSender<TrackEvent>> {
    let partition_index = self.get_subscriber_partition_index(subscriber_id);
    let senders = self.subscriber_senders[partition_index].read().await;
    senders.get(&subscriber_id).cloned()
  }

  pub async fn add_subscription(
    &mut self,
    subscriber: Arc<MOQTClient>,
    subscribe_message: Subscribe,
    cache: super::track_cache::TrackCache,
  ) -> Result<(), anyhow::Error> {
    let connection_id = { subscriber.connection_id };

    info!(
      "Adding subscription for subscriber_id: {} to track: {} subscription message: {:?}",
      connection_id, self.track_alias, subscribe_message
    );

    // Create a separate unbounded channel for this subscriber
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<TrackEvent>();

    let subscription = Subscription::new(
      self.track_alias,
      subscribe_message,
      subscriber.clone(),
      event_rx,
      cache,
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

    // Store the sender for this subscriber in the appropriate partition
    let partition_index = self.get_subscriber_partition_index(connection_id);
    let mut senders = self.subscriber_senders[partition_index].write().await;
    senders.insert(connection_id, event_tx);

    Ok(())
  }

  // return the subscription for the client
  // subscriber_id is the connection id of the client
  pub async fn get_subscription(&self, subscriber_id: usize) -> Option<Arc<RwLock<Subscription>>> {
    debug!(
      "Getting subscription for subscriber_id: {} from track: {}",
      subscriber_id, self.track_alias
    );
    let subscriptions = self.subscriptions.read().await;
    // find the subscription by subscriber_id and finish it
    subscriptions.get(&subscriber_id).cloned()
  }

  pub async fn remove_subscription(&self, subscriber_id: usize) {
    info!(
      "Removing subscription for subscriber_id: {} from track: {}",
      subscriber_id, self.track_alias
    );
    let mut subscriptions = self.subscriptions.write().await;
    let sub = subscriptions.remove(&subscriber_id);
    drop(subscriptions);

    // find the subscription by subscriber_id and finish it
    if let Some(subscription) = sub {
      let sub = subscription.write().await;
      sub.finish().await;
    }

    // Remove and dispose the sender for this subscriber from the appropriate partition
    let partition_index = self.get_subscriber_partition_index(subscriber_id);
    let mut senders = self.subscriber_senders[partition_index].write().await;
    if let Some(_sender) = senders.remove(&subscriber_id) {
      // The sender is automatically dropped here, which closes the channel
      info!(
        "Disposed sender for subscriber_id: {} from track: {}",
        subscriber_id, self.track_alias
      );
    }
    drop(senders);
  }

  // Send event to all subscribers
  pub async fn send_event_to_subscribers(
    &self,
    event: TrackEvent,
  ) -> Result<Vec<usize>, anyhow::Error> {
    let mut failed_subscribers = Vec::new();
    let mut total_subscribers = 0;

    // Iterate through all partitions
    for partition in self.subscriber_senders.iter() {
      let senders = partition.read().await;

      if !senders.is_empty() {
        total_subscribers += senders.len();

        for (subscriber_id, sender) in senders.iter() {
          if let Err(e) = sender.send(event.clone()) {
            error!(
              "Failed to send event to subscriber {}: {}",
              subscriber_id, e
            );
            failed_subscribers.push(*subscriber_id);
          }
        }
      }
    }

    if !failed_subscribers.is_empty() {
      error!(
        "{:?} event sent to {} subscribers, {} failed for track: {}",
        event,
        total_subscribers - failed_subscribers.len(),
        failed_subscribers.len(),
        self.track_alias
      );
    } else if total_subscribers > 0 {
      debug!(
        "{:?} event sent successfully to {} subscribers for track: {}",
        event, total_subscribers, self.track_alias
      );
    }

    Ok(failed_subscribers)
  }
}
