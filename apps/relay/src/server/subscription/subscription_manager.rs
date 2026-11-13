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

use crate::server::inspect;
use crate::server::{subscription::SubscriptionOrigin, track::ActiveSubgroupHeaderMap};

use crate::server::config::AppConfig;
use crate::server::session::client::MOQTClient;
use crate::server::subscription::Subscription;
use crate::server::track::TrackEvent;
use anyhow::Result;
use moqtail::model::data::full_track_name::FullTrackName;
use std::{
  collections::{BTreeMap, HashMap, HashSet},
  sync::{
    Arc, RwLock as SyncRwLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
  },
  time::Instant,
};
use tokio::sync::RwLock;
#[cfg(test)]
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, info, trace, warn};

pub type SubscriptionId = (usize, u64); // connection_id and request_id
/// An event and when it was queued, so the subscriber can tell how late it is.
/// When an event passed the relay before reaching a subscription's queue.
#[derive(Debug, Clone, Copy)]
pub struct EventTimes {
  /// When the object was read from the publisher; `None` when the relay is built
  /// without measuring processing time.
  pub arrived: Option<Instant>,
  /// When the event was handed to the subscribers' queues.
  pub queued: Instant,
}

pub type QueuedEvent = (EventTimes, TrackEvent);
pub type SubscriberSenderMap =
  HashMap<SubscriptionId, (UnboundedSender<QueuedEvent>, Arc<SubscriberQueue>)>;

/// What the fan-out and a subscription share about the subscription's queue.
#[derive(Debug, Default)]
pub(crate) struct SubscriberQueue {
  /// Payload bytes queued and not yet taken by the subscription.
  pub bytes: AtomicUsize,
  /// Groups below this are skipped: their Objects were dropped or arrived too late.
  pub skip_below_group: AtomicU64,
}

impl SubscriberQueue {
  /// Skips the rest of `group`. True if it was not skipped already.
  pub fn skip_group(&self, group: u64) -> bool {
    self
      .skip_below_group
      .fetch_max(group.saturating_add(1), Ordering::Relaxed)
      <= group
  }

  pub fn skips(&self, group: u64) -> bool {
    group < self.skip_below_group.load(Ordering::Relaxed)
  }
}

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionManager {
  relay_track_id: u64,
  full_track_name: FullTrackName,
  subscriptions: Arc<RwLock<BTreeMap<SubscriptionId, Arc<RwLock<Subscription>>>>>,
  subscriber_request_ids: Arc<RwLock<BTreeMap<usize, HashSet<u64>>>>,
  /// A plain lock: the fan-out sends without awaiting, so it is never held across one.
  subscriber_senders: Arc<SyncRwLock<SubscriberSenderMap>>,
  log_folder: String,
  config: &'static AppConfig,
}

/// Counts an object not queued for a subscriber that had too much waiting.
fn count_queue_drop() {
  if let Some(c) = inspect::counters() {
    c.dropped_queue_full.add(1);
  }
}

impl SubscriptionManager {
  #[cfg(feature = "observability")]
  /// Every subscription to the track, for reporting.
  pub async fn all(&self) -> Vec<Arc<RwLock<Subscription>>> {
    self.subscriptions.read().await.values().cloned().collect()
  }

  pub fn new(
    relay_track_id: u64,
    full_track_name: FullTrackName,
    log_folder: String,
    config: &'static AppConfig,
  ) -> Self {
    SubscriptionManager {
      relay_track_id,
      full_track_name,
      subscriptions: Arc::new(RwLock::new(BTreeMap::new())),
      subscriber_request_ids: Arc::new(RwLock::new(BTreeMap::new())),
      subscriber_senders: Arc::default(),
      log_folder,
      config,
    }
  }

  /// Takes the track's events without a subscription behind them.
  #[cfg(test)]
  pub(crate) fn add_test_sender(
    &self,
    id: SubscriptionId,
  ) -> (UnboundedReceiver<QueuedEvent>, Arc<SubscriberQueue>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let queue = Arc::new(SubscriberQueue::default());
    self.senders_mut().insert(id, (tx, queue.clone()));
    (rx, queue)
  }

  fn senders_mut(&self) -> std::sync::RwLockWriteGuard<'_, SubscriberSenderMap> {
    self
      .subscriber_senders
      .write()
      .unwrap_or_else(|e| e.into_inner())
  }

  pub async fn get_all_subscriptions(&self) -> Vec<Arc<RwLock<Subscription>>> {
    let subs = self.subscriptions.read().await;
    subs.values().cloned().collect()
  }

  pub async fn add_subscription(
    &self,
    subscriber: Arc<MOQTClient>,
    origin_message: SubscriptionOrigin,
    cache: crate::server::track::track_cache::TrackCache,
    active_subgroup_headers: ActiveSubgroupHeaderMap,
  ) -> Result<Arc<RwLock<Subscription>>, anyhow::Error> {
    let connection_id = { subscriber.connection_id };

    info!(
      "Adding subscription for subscriber_id={} to relay_track_id={} subscription message: {:?}",
      connection_id, self.relay_track_id, &origin_message
    );

    // Create a separate unbounded channel for this subscriber
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<QueuedEvent>();
    let queue = Arc::new(SubscriberQueue::default());

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
      queue.clone(),
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

    self
      .senders_mut()
      .insert(subscription_id, (event_tx, queue));

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

    // Dropping the sender closes the subscription's channel.
    let removed = self.senders_mut().remove(subscription_id);
    if removed.is_some() {
      info!(
        "Disposed sender for subscription_id={:?} from relay_track_id={}",
        subscription_id, self.relay_track_id
      );
    }
  }

  /// Takes every subscription off the track without cancelling it, so one already
  /// told the track ended still delivers what it has queued and finishes on its own,
  /// while nothing more reaches it.
  pub async fn detach_all(&self) {
    let detached = std::mem::take(&mut *self.subscriptions.write().await);
    self.subscriber_request_ids.write().await.clear();
    self.senders_mut().clear();
    info!(
      "Detached {} subscription(s) from relay_track_id={}",
      detached.len(),
      self.relay_track_id
    );
    for ((_, request_id), subscription) in detached {
      let subscriber = subscription.read().await.subscriber();
      subscriber.subscriptions.remove(request_id).await;
      subscriber.downstream_requests.remove(request_id).await;
    }
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

  /// Sends an event that did not come from a publisher's object, such as a closed
  /// stream, to all subscribers.
  pub async fn send_event_to_subscribers(
    &self,
    event: TrackEvent,
  ) -> Result<Vec<SubscriptionId>, anyhow::Error> {
    self.send_object_event(event, None).await
  }

  /// Sends an event to all subscribers. `arrived` is when its object was read from
  /// the publisher.
  pub async fn send_object_event(
    &self,
    event: TrackEvent,
    arrived: Option<Instant>,
  ) -> Result<Vec<SubscriptionId>, anyhow::Error> {
    let mut failed_subscriptions = Vec::new();
    let mut total_subscriptions = 0;
    let queued_at = Instant::now();
    let times = EventTimes {
      arrived,
      queued: queued_at,
    };
    let bytes = event.queued_bytes();
    let max_bytes = self.config.max_subscriber_queue_bytes;

    {
      let senders = self
        .subscriber_senders
        .read()
        .unwrap_or_else(|e| e.into_inner());
      total_subscriptions += senders.len();
      for (subscription_id, (sender, queue)) in senders.iter() {
        // Never blocks the publisher: a subscriber with too much queued loses the rest
        // of the group instead, and picks up again at a later one.
        if let Some(group) = event.object_group() {
          // The subscription drops these anyway; queuing them only keeps it at the cap.
          if queue.skips(group) {
            count_queue_drop();
            continue;
          }
          // An empty queue takes any Object, so one larger than the cap still goes out.
          let queued = queue.bytes.load(Ordering::Relaxed);
          if max_bytes > 0 && queued > 0 && queued + bytes > max_bytes {
            if queue.skip_group(group) {
              warn!(
                "Subscription {:?} has over {} bytes queued on relay_track_id={}; skipping the rest of group {}",
                subscription_id, max_bytes, self.relay_track_id, group
              );
            }
            count_queue_drop();
            continue;
          }
        }
        queue.bytes.fetch_add(bytes, Ordering::Relaxed);
        if let Err(e) = sender.send((times, event.clone())) {
          queue.bytes.fetch_sub(bytes, Ordering::Relaxed);
          error!(
            "Failed to send event to subscription {:?}: {}",
            subscription_id, e
          );
          failed_subscriptions.push(*subscription_id);
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::server::stream_id::StreamId;
  use bytes::Bytes;
  use moqtail::model::common::location::Location;
  use moqtail::model::data::constant::{ObjectForwardingPreference, ObjectStatus};
  use moqtail::model::data::object::Object;

  fn object(group: u64, id: u64) -> TrackEvent {
    TrackEvent::SubgroupObject {
      stream_id: StreamId::new_subgroup(1, group, Some(0)),
      object: Arc::new(Object {
        track_alias: 1,
        location: Location::new(group, id),
        publisher_priority: 128,
        forwarding_preference: ObjectForwardingPreference::Subgroup,
        subgroup_id: Some(0),
        status: ObjectStatus::Normal,
        properties: None,
        payload: Some(Bytes::from(vec![0; 100])),
      }),
      header_info: None,
    }
  }

  fn location(event: &TrackEvent) -> Option<Location> {
    match event {
      TrackEvent::SubgroupObject { object, .. } => Some(object.location.clone()),
      _ => None,
    }
  }

  #[tokio::test]
  async fn a_full_queue_skips_the_rest_of_the_group_but_not_control_events() {
    let config = Box::leak(Box::new(AppConfig {
      max_subscriber_queue_bytes: 250,
      ..Default::default()
    }));
    let name = FullTrackName::try_new("meet/room1", "video").unwrap();
    let manager = SubscriptionManager::new(1, name, String::new(), config);
    let (mut rx, queue) = manager.add_test_sender((1, 0));

    for id in 0..3 {
      manager
        .send_event_to_subscribers(object(1, id))
        .await
        .unwrap();
    }
    let closed = TrackEvent::StreamClosed {
      stream_id: StreamId::new_subgroup(1, 1, Some(0)),
    };
    manager.send_event_to_subscribers(closed).await.unwrap();
    assert_eq!(
      queue.bytes.load(Ordering::Relaxed),
      200,
      "the third Object is dropped"
    );
    assert!(queue.skips(1));

    // The subscriber catches up: the rest of group 1 is not even queued, group 2 goes out.
    queue.bytes.store(0, Ordering::Relaxed);
    manager
      .send_event_to_subscribers(object(1, 3))
      .await
      .unwrap();
    manager
      .send_event_to_subscribers(object(2, 0))
      .await
      .unwrap();
    assert!(!queue.skips(2));

    let mut got = Vec::new();
    while let Ok((_, event)) = rx.try_recv() {
      got.push(location(&event));
    }
    assert_eq!(
      got,
      vec![
        Some(Location::new(1, 0)),
        Some(Location::new(1, 1)),
        None,
        Some(Location::new(2, 0)),
      ]
    );
  }

  #[tokio::test]
  async fn an_object_larger_than_the_cap_goes_out_when_the_queue_is_empty() {
    let config = Box::leak(Box::new(AppConfig {
      max_subscriber_queue_bytes: 50,
      ..Default::default()
    }));
    let name = FullTrackName::try_new("meet/room1", "video").unwrap();
    let manager = SubscriptionManager::new(1, name, String::new(), config);
    let (mut rx, queue) = manager.add_test_sender((1, 0));

    // 100 bytes each against a 50-byte cap: the first fits an empty queue, the
    // second does not fit behind it.
    for id in 0..2 {
      manager
        .send_event_to_subscribers(object(1, id))
        .await
        .unwrap();
    }
    assert_eq!(queue.bytes.load(Ordering::Relaxed), 100);
    assert!(queue.skips(1));
    assert_eq!(
      location(&rx.try_recv().unwrap().1),
      Some(Location::new(1, 0))
    );
    assert!(rx.try_recv().is_err());
  }

  #[test]
  fn skipping_a_group_reports_only_the_first_time() {
    let queue = SubscriberQueue::default();
    assert!(!queue.skips(4));
    assert!(queue.skip_group(4));
    assert!(!queue.skip_group(4));
    assert!(!queue.skip_group(3), "an older group is already covered");
    assert!(queue.skips(3) && queue.skips(4) && !queue.skips(5));
  }
}
