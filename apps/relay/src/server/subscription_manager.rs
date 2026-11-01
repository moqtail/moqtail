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

use crate::server::{subscription::SubscriptionOrigin, track::ActiveSubgroupHeaderMap};

use super::client::MOQTClient;
use super::config::AppConfig;
use super::subscription::Subscription;
use super::track::TrackEvent;
use super::utils;
use anyhow::Result;
use moqtail::model::data::full_track_name::FullTrackName;
use std::{
  collections::{BTreeMap, HashMap, HashSet},
  sync::Arc,
};
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, info, trace};

/// Number of partitions for subscriber senders management to reduce lock contention.
/// Each partition contains a separate HashMap protected by its own RwLock.
/// Higher values reduce contention but increase memory overhead.
/// Should be a power of 2 for optimal modulo performance.
const SUBSCRIBER_PARTITION_COUNT: usize = 16;
pub type SubscriptionId = (usize, u64); // connection_id and request_id
pub type SubscriberSenderMap = HashMap<SubscriptionId, UnboundedSender<TrackEvent>>;
pub type SubscriberSenderLock = Arc<RwLock<SubscriberSenderMap>>;
pub type SubscriberSenderList = Vec<SubscriberSenderLock>;

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionManager {
  relay_track_id: u64,
  full_track_name: FullTrackName,
  subscriptions: Arc<RwLock<BTreeMap<SubscriptionId, Arc<RwLock<Subscription>>>>>,
  subscriber_request_ids: Arc<RwLock<BTreeMap<usize, HashSet<u64>>>>,
  subscriber_senders: Arc<SubscriberSenderList>,
  log_folder: String,
  config: &'static AppConfig,
}

impl SubscriptionManager {
  pub fn new(
    relay_track_id: u64,
    full_track_name: FullTrackName,
    log_folder: String,
    config: &'static AppConfig,
  ) -> Self {
    // Initialize partitioned subscriber senders
    let mut subscriber_senders = Vec::with_capacity(SUBSCRIBER_PARTITION_COUNT);
    for _ in 0..SUBSCRIBER_PARTITION_COUNT {
      subscriber_senders.push(Arc::new(RwLock::new(HashMap::new())));
    }

    SubscriptionManager {
      relay_track_id,
      full_track_name,
      subscriptions: Arc::new(RwLock::new(BTreeMap::new())),
      subscriber_request_ids: Arc::new(RwLock::new(BTreeMap::new())),
      subscriber_senders: Arc::from(subscriber_senders),
      log_folder,
      config,
    }
  }

  /// Calculate the partition index for subscriber distribution across buckets.
  /// This method implements a load balancing strategy to distribute subscribers
  /// across multiple buckets to improve performance and reduce contention.
  fn get_subscriber_partition_index(&self, subscription_id: &SubscriptionId) -> usize {
    // Convert to bytes for fnv_hash function
    let value_bytes = [
      (subscription_id.0 as u64).to_le_bytes(),
      subscription_id.1.to_le_bytes(),
    ]
    .concat();
    (utils::fnv_hash(&value_bytes) % SUBSCRIBER_PARTITION_COUNT as u64) as usize
  }

  /// Get the subscriber sender for a specific subscriber
  async fn _get_subscriber_sender(
    &self,
    subscription_id: &SubscriptionId,
  ) -> Option<UnboundedSender<TrackEvent>> {
    let partition_index = self.get_subscriber_partition_index(subscription_id);
    let senders = self.subscriber_senders[partition_index].read().await;
    senders.get(subscription_id).cloned()
  }

  pub async fn get_all_subscriptions(&self) -> Vec<Arc<RwLock<Subscription>>> {
    let subs = self.subscriptions.read().await;
    subs.values().cloned().collect()
  }

  pub async fn add_subscription(
    &self,
    subscriber: Arc<MOQTClient>,
    origin_message: SubscriptionOrigin,
    cache: super::track_cache::TrackCache,
    active_subgroup_headers: ActiveSubgroupHeaderMap,
  ) -> Result<Arc<RwLock<Subscription>>, anyhow::Error> {
    let connection_id = { subscriber.connection_id };

    info!(
      "Adding subscription for subscriber_id={} to relay_track_id={} subscription message: {:?}",
      connection_id, self.relay_track_id, &origin_message
    );

    // Create a separate unbounded channel for this subscriber
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<TrackEvent>();

    // A pushed track's alias is the one its PUBLISH already gave the subscriber.
    let track_alias = match &origin_message {
      SubscriptionOrigin::Publish(publish) => publish.track_alias,
      SubscriptionOrigin::Subscribe(_) => subscriber.assign_track_alias(self.relay_track_id),
    };

    let subscription = Subscription::new(
      self.relay_track_id,
      track_alias,
      self.full_track_name.clone(),
      origin_message.clone(),
      subscriber.clone(),
      event_rx,
      cache,
      connection_id,
      self.log_folder.clone(),
      self.config,
      active_subgroup_headers,
    );

    let subscription_id = (connection_id, origin_message.request_id());

    let mut subscriptions = self.subscriptions.write().await;
    if subscriptions.contains_key(&subscription_id) {
      error!(
        "Subscriber with subscription_id={:?} already exists in relay_track_id={}",
        subscription_id, self.relay_track_id
      );
      return Err(anyhow::anyhow!("Subscription with same ID already exists"));
    }

    let subscription = Arc::new(RwLock::new(subscription));
    subscriptions.insert(subscription_id, subscription.clone());

    let mut sub_req_id_map = self.subscriber_request_ids.write().await;
    sub_req_id_map
      .entry(subscription_id.0)
      .and_modify(|s| _ = s.insert(subscription_id.1))
      .or_insert(HashSet::from([subscription_id.1]));

    // Store the sender for this subscriber in the appropriate partition
    let partition_index = self.get_subscriber_partition_index(&subscription_id);
    let mut senders = self.subscriber_senders[partition_index].write().await;
    senders.insert(subscription_id, event_tx);

    Ok(subscription)
  }

  pub async fn has_any_subscription(&self, subscriber_id: usize) -> bool {
    if let Some(s) = self.subscriber_request_ids.read().await.get(&subscriber_id) {
      !s.is_empty()
    } else {
      false
    }
  }

  pub async fn exists(&self, subscription_id: &SubscriptionId) -> bool {
    if let Some(s) = self
      .subscriber_request_ids
      .read()
      .await
      .get(&subscription_id.0)
    {
      s.contains(&subscription_id.1)
    } else {
      false
    }
  }

  // return the subscription for the client
  // subscriber_id is the connection id of the client
  pub async fn get_subscription(
    &self,
    subscription_id: &SubscriptionId,
  ) -> Option<Arc<RwLock<Subscription>>> {
    trace!(
      "Getting subscription for subscription_id={:?} from relay_track_id={}",
      subscription_id, self.relay_track_id
    );
    let subscriptions = self.subscriptions.read().await;
    // find the subscription by subscriber_id and finish it
    subscriptions.get(subscription_id).cloned()
  }

  pub async fn remove_subscription(&self, subscription_id: &SubscriptionId) {
    info!(
      "Removing subscription for subscription_id={:?} from relay_track_id={}",
      subscription_id, self.relay_track_id
    );

    let mut subscriptions = self.subscriptions.write().await;
    let sub = subscriptions.remove(subscription_id);

    let mut sub_req_ids = self.subscriber_request_ids.write().await;
    if let Some(s) = sub_req_ids.get_mut(&subscription_id.0) {
      s.remove(&subscription_id.1);
      if s.is_empty() {
        sub_req_ids.remove(&subscription_id.0);
      }
    }

    drop(subscriptions);

    // find the subscription by subscriber_id and cancel it. Every caller of this
    // reaches it because the subscriber went away, so its streams are reset.
    if let Some(subscription) = sub {
      let sub = subscription.write().await;
      sub.cancel().await;
    }

    // Remove and dispose the sender for this subscriber from the appropriate partition
    let partition_index = self.get_subscriber_partition_index(subscription_id);
    let mut senders = self.subscriber_senders[partition_index].write().await;
    if let Some(_sender) = senders.remove(subscription_id) {
      // The sender is automatically dropped here, which closes the channel
      info!(
        "Disposed sender for subscription_id={:?} from relay_track_id={}",
        subscription_id, self.relay_track_id
      );
    }
    drop(senders);
  }

  /// Takes every subscription off the track without cancelling it, so one already
  /// told the track ended still delivers what it has queued and finishes on its own,
  /// while nothing more reaches it.
  pub async fn detach_all(&self) {
    let detached = std::mem::take(&mut *self.subscriptions.write().await);
    self.subscriber_request_ids.write().await.clear();
    for partition in self.subscriber_senders.iter() {
      partition.write().await.clear();
    }
    info!(
      "Detached {} subscription(s) from relay_track_id={}",
      detached.len(),
      self.relay_track_id
    );
  }

  /// Removes every subscription the subscriber holds.
  pub async fn remove_subscriptions(&self, subscriber_id: usize) {
    info!(
      "Removing all subscriptions for subscriber_id={} from relay_track_id={}",
      subscriber_id, self.relay_track_id
    );
    let request_ids = self
      .subscriber_request_ids
      .read()
      .await
      .get(&subscriber_id)
      .cloned();
    for request_id in request_ids.into_iter().flatten() {
      self.remove_subscription(&(subscriber_id, request_id)).await;
    }
  }

  pub async fn subscriber_count(&self) -> usize {
    self.subscriptions.read().await.len()
  }

  // Send event to all subscribers
  pub async fn send_event_to_subscribers(
    &self,
    event: TrackEvent,
  ) -> Result<Vec<SubscriptionId>, anyhow::Error> {
    let mut failed_subscriptions = Vec::new();
    let mut total_subscriptions = 0;

    // Iterate through all partitions
    for partition in self.subscriber_senders.iter() {
      let senders = partition.read().await;

      if !senders.is_empty() {
        total_subscriptions += senders.len();

        for (subscription_id, sender) in senders.iter() {
          if let Err(e) = sender.send(event.clone()) {
            error!(
              "Failed to send event to subscription {:?}: {}",
              subscription_id, e
            );
            failed_subscriptions.push(*subscription_id);
          }
        }
      }
    }

    if !failed_subscriptions.is_empty() {
      error!(
        "{:?} event sent to {} subscribers, {} failed for relay_track_id={}",
        event,
        total_subscriptions - failed_subscriptions.len(),
        failed_subscriptions.len(),
        self.relay_track_id
      );
    } else if total_subscriptions > 0 {
      trace!(
        "{:?} event sent successfully to {} subscribers for relay_track_id={}",
        event, total_subscriptions, self.relay_track_id
      );
    }

    Ok(failed_subscriptions)
  }
}
