use super::track_cache::TrackCache;
use crate::server::client::MOQTClient;
use crate::server::subscription::Subscription;
use crate::server::utils;
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::object::Object;
use moqtail::{model::common::tuple::Tuple, transport::data_stream_handler::HeaderInfo};
use std::time::Instant;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub enum TrackEvent {
  Header { header: HeaderInfo },
  Object { stream_id: String, object: Object },
  StreamClosed { stream_id: String },
  PublisherDisconnected { reason: String },
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
}

// TODO: this track implementation should be static? At least
// its lifetime should be same as the server's lifetime
impl Track {
  pub fn new(
    track_alias: u64,
    track_namespace: Tuple,
    track_name: String,
    cache_size: usize,
    publisher_connection_id: usize,
  ) -> Self {
    Track {
      track_alias,
      track_namespace,
      track_name,
      subscriptions: Arc::new(RwLock::new(BTreeMap::new())),
      publisher_connection_id,
      cache: TrackCache::new(track_alias, cache_size),
      subscriber_senders: Arc::new(RwLock::new(BTreeMap::new())),
      largest_location: Arc::new(RwLock::new(Location::new(0, 0))),
    }
  }

  pub async fn add_subscription(
    &mut self,
    subscriber: Arc<RwLock<MOQTClient>>,
    subscribe_message: Subscribe,
  ) -> Result<(), anyhow::Error> {
    let connection_id = { subscriber.read().await.connection_id };

    info!(
      "Adding subscription for subscriber_id: {} to track: {}",
      connection_id, self.track_alias
    );

    // Create a separate unbounded channel for this subscriber
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

    let subscription = Subscription::new(
      subscribe_message,
      subscriber.clone(),
      event_rx,
      self.cache.clone(),
      connection_id,
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

  pub async fn new_header(&self, header: &HeaderInfo) -> Result<()> {
    let header_event = TrackEvent::Header {
      header: header.clone(),
    };

    self.send_event_to_subscribers(header_event).await?;

    Ok(())
  }

  pub async fn new_object(&self, stream_id: String, object: &Object) -> Result<(), anyhow::Error> {
    debug!(
      "new_object: track: {:?} location: {:?} stream_id: {} diff_ms: {}",
      object.track_alias,
      object.location,
      &stream_id,
      (Instant::now() - *utils::BASE_TIME).as_millis()
    );

    if let Ok(fetch_object) = object.clone().try_into_fetch() {
      self.cache.add_object(fetch_object).await;
      let object_event = TrackEvent::Object {
        stream_id: stream_id.clone(),
        object: object.clone(),
      };

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

      self.send_event_to_subscribers(object_event).await?;
      Ok(())
    } else {
      error!(
        "new_object: track: {:?} location: {:?} stream_id: {} diff_ms: {} object: {:?}",
        object.track_alias,
        object.location,
        stream_id,
        (Instant::now() - *utils::BASE_TIME).as_millis(),
        object
      );
      Err(anyhow::anyhow!("Object is not a fetch object"))
    }
  }

  pub async fn stream_closed(&self, stream_id: String) -> Result<(), anyhow::Error> {
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
