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

mod delivery;
pub(crate) mod fill;
pub(crate) mod filter_constructor;
pub(crate) mod prefix_subscription;
mod stream_writer;
pub(crate) mod subscription_manager;
pub(crate) mod subscription_state;
mod switch;

use stream_writer::for_mid_subgroup_stream;

use crate::server::config::AppConfig;
use crate::server::logging::object_logger::ObjectLogger;
use crate::server::message_handlers::fetch_handler::FetchStop;
use crate::server::session::client::MOQTClient;
use crate::server::ssts::gate::ForwardGate;
use crate::server::stream_id::StreamId;
use crate::server::subscription::subscription_manager::{QueuedEvent, SubscriberQueue};
use crate::server::subscription::subscription_state::SubscriptionState;
use crate::server::subscription::subscription_state::live_start_location;
use crate::server::track::ActiveSubgroupHeaderMap;
use crate::server::track::track_cache::TrackCache;
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish::Publish;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::control::request_update::RequestUpdate;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::error::StreamResetCode;
use moqtail::model::parameter::combined_filter::CombinedFilter;
use moqtail::model::parameter::message_parameter::{
  MessageParameter, apply_message_parameter_update, include_properties,
};
use moqtail::model::property::track_property::TrackProperty;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::watch;
use tracing::{debug, info, trace, warn};

#[derive(Debug, Clone)]
pub enum SubscriptionOrigin {
  Subscribe(Subscribe),
  Publish(Publish),
}

impl SubscriptionOrigin {
  pub fn request_id(&self) -> u64 {
    match self {
      SubscriptionOrigin::Subscribe(s) => s.request_id,
      SubscriptionOrigin::Publish(p) => p.request_id,
    }
  }
}
impl From<Subscribe> for SubscriptionOrigin {
  fn from(msg: Subscribe) -> Self {
    SubscriptionOrigin::Subscribe(msg)
  }
}

impl From<Publish> for SubscriptionOrigin {
  fn from(msg: Publish) -> Self {
    SubscriptionOrigin::Publish(msg)
  }
}

/// The publisher priority assumed where a stream carries no single Object's own.
pub(crate) const DEFAULT_PUBLISHER_PRIORITY: u8 = 128;

#[derive(Debug, Clone)]
pub struct Subscription {
  pub request_id: u64,
  relay_track_id: u64,
  /// The Track Alias this subscriber knows the track by on this subscription. It is
  /// the track's relay id or one of the subscription's own, as `track_alias_mode`
  /// decides; `relay_track_id` stays the relay's internal name for the track.
  pub track_alias: u64,
  pub full_track_name: FullTrackName,
  pub subscription_state: Arc<RwLock<SubscriptionState>>,
  subscriber: Arc<MOQTClient>,
  event_rx: Arc<Mutex<Option<UnboundedReceiver<QueuedEvent>>>>,
  queue: Arc<SubscriberQueue>,
  send_stream_last_object_ids: Arc<RwLock<HashMap<StreamId, Option<u64>>>>,
  /// Monotonic count of data streams opened for this subscription, including
  /// empty subgroups. Reported as PUBLISH_DONE Stream Count.
  opened_stream_count: Arc<AtomicU64>,
  finished: Arc<AtomicBool>,
  #[allow(dead_code)]
  cache: TrackCache,
  client_connection_id: usize,
  object_logger: ObjectLogger,
  config: &'static AppConfig,
  /// Set on the activating subscription of a switch, until it delivers the
  /// object that completes it.
  pending_switch: Arc<AtomicBool>,
  /// Subgroup header cached while forward=false. Cleared when forward becomes true (stream opened)
  /// or when a new group starts (old group ended without forward ever becoming true).
  pending_header: Arc<Mutex<Option<(StreamId, HeaderInfo)>>>,
  /// Shared map of open publisher subgroup streams and their original subgroup header.
  /// Used to open a QUIC send stream for a new mid-group subscriber with the exact
  /// original header rather than a synthesized one.
  active_subgroup_headers: ActiveSubgroupHeaderMap,
  /// Headers of streams whose first object this subscription skipped (filtered out,
  /// or before the start location). If a later object of the stream is sent, the
  /// stream is opened with this header. Removed when the stream closes.
  ///
  /// `active_subgroup_headers` can't be used for this. The track removes a header as
  /// soon as the publisher closes the stream, which can happen before this
  /// subscription has processed the rest of the stream's objects.
  skipped_stream_headers: Arc<Mutex<HashMap<StreamId, HeaderInfo>>>,
  /// Set once the control message carrying this subscriber's track alias has gone out
  /// (SUBSCRIBE_OK, or PUBLISH when the relay pushes). Data streams carry only the
  /// alias, so a subscriber that gets one first cannot place it and drops the objects.
  /// Forwarding waits for this, and the queued Objects follow in order.
  alias_announced: Arc<AtomicBool>,
  alias_announced_notify: Arc<Notify>,
  /// Fill fetch streams still delivering, keyed by the request that asked for the
  /// fill. Held so Forward State 0 and cancellation can stop them.
  fill_streams: Arc<RwLock<HashMap<u64, watch::Sender<FetchStop>>>>,
  ssts_gate: Arc<ForwardGate>,
}

#[allow(clippy::too_many_arguments)]
impl Subscription {
  fn create_instance(
    relay_track_id: u64,
    track_alias: u64,
    full_track_name: FullTrackName,
    request_id: u64,
    origin_message: SubscriptionOrigin,
    subscriber: Arc<MOQTClient>,
    event_rx: Arc<Mutex<Option<UnboundedReceiver<QueuedEvent>>>>,
    queue: Arc<SubscriberQueue>,
    cache: TrackCache,
    client_connection_id: usize,
    log_folder: String,
    config: &'static AppConfig,
    active_subgroup_headers: ActiveSubgroupHeaderMap,
  ) -> Self {
    Self {
      ssts_gate: Arc::new(ForwardGate::new(full_track_name.clone(), relay_track_id)),
      relay_track_id,
      track_alias,
      full_track_name,
      request_id,
      subscription_state: Arc::new(RwLock::new(origin_message.into())),
      subscriber,
      event_rx,
      queue,
      send_stream_last_object_ids: Arc::new(RwLock::new(HashMap::new())),
      opened_stream_count: Arc::new(AtomicU64::new(0)),
      finished: Arc::new(AtomicBool::new(false)),
      cache,
      client_connection_id,
      object_logger: ObjectLogger::new(log_folder),
      config,
      pending_switch: Arc::new(AtomicBool::new(false)),
      pending_header: Arc::new(Mutex::new(None)),
      active_subgroup_headers,
      skipped_stream_headers: Arc::new(Mutex::new(HashMap::new())),
      alias_announced: Arc::new(AtomicBool::new(false)),
      alias_announced_notify: Arc::new(Notify::new()),
      fill_streams: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  /// Tracks a fill fetch stream that has started delivering.
  pub async fn register_fill_stream(&self, request_id: u64, cancel: watch::Sender<FetchStop>) {
    self.fill_streams.write().await.insert(request_id, cancel);
  }

  pub async fn unregister_fill_stream(&self, request_id: u64) {
    self.fill_streams.write().await.remove(&request_id);
  }

  /// Stops every fill fetch stream still delivering for this subscription.
  pub async fn stop_fill_streams(&self, reason: FetchStop) {
    for (_, cancel) in self.fill_streams.write().await.drain() {
      let _ = cancel.send(reason);
    }
  }

  /// A fill fetch stream counts towards PUBLISH_DONE Stream Count like any other.
  pub fn note_fill_stream_opened(&self) {
    self.opened_stream_count.fetch_add(1, Ordering::Relaxed);
  }

  /// Pins down where a live filter starts, now that the track's largest Object is
  /// known. Next Object and Relative Start say where to begin only in terms of
  /// that, so until this runs the subscription starts at `{0,0}` and delivers
  /// whatever arrives — mid-group for a start at the Next Group, which is the one
  /// thing it asks not to happen.
  pub async fn resolve_live_start(&self, largest: Option<Location>) {
    let filter = self.subscription_state.read().await.location_filter.clone();
    let Some(start) = live_start_location(&filter, largest) else {
      return;
    };
    debug!(
      "Start location for subscriber={} relay_track_id={} resolved to {:?} by {:?}",
      self.client_connection_id, self.relay_track_id, start, filter
    );
    self.subscription_state.write().await.start_location = Some(start);
  }

  /// Called once the subscriber has been sent its track alias, releasing forwarding.
  pub fn mark_alias_announced(&self) {
    self.alias_announced.store(true, Ordering::Release);
    self.alias_announced_notify.notify_waiters();
  }

  /// Waits for the alias to be announced, giving up after `downstream_alias_timeout`
  /// so a subscription that never gets one still drains rather than queueing forever.
  async fn wait_for_alias_announced(&self) {
    if self.alias_announced.load(Ordering::Acquire) {
      return;
    }
    // Registered before the second check, so a notify landing between the two is not
    // lost. Waking can only come from mark_alias_announced, which sets the flag first.
    let notified = self.alias_announced_notify.notified();
    if self.alias_announced.load(Ordering::Acquire) || self.is_finished().await {
      return;
    }
    if tokio::time::timeout(self.config.downstream_alias_timeout, notified)
      .await
      .is_err()
    {
      warn!(
        "no track alias sent to subscriber {} within {:?}; forwarding anyway",
        self.client_connection_id, self.config.downstream_alias_timeout
      );
    }
  }

  pub fn new(
    relay_track_id: u64,
    track_alias: u64,
    full_track_name: FullTrackName,
    origin_message: SubscriptionOrigin,
    subscriber: Arc<MOQTClient>,
    event_rx: UnboundedReceiver<QueuedEvent>,
    queue: Arc<SubscriberQueue>,
    cache: TrackCache,
    client_connection_id: usize,
    log_folder: String,
    config: &'static AppConfig,
    active_subgroup_headers: ActiveSubgroupHeaderMap,
  ) -> Self {
    let event_rx = Arc::new(Mutex::new(Some(event_rx)));
    let sub = Self::create_instance(
      relay_track_id,
      track_alias,
      full_track_name,
      origin_message.request_id(),
      origin_message,
      subscriber,
      event_rx,
      queue,
      cache.clone(),
      client_connection_id,
      log_folder,
      config,
      active_subgroup_headers,
    );

    info!(
      "Created new Subscription instance for subscriber={} relay_track_id={} subscription state: {:?}",
      client_connection_id, relay_track_id, sub.subscription_state
    );

    let mut instance = sub.clone();

    tokio::spawn(async move {
      // Nothing may go out before the subscriber knows the alias those streams carry.
      instance.wait_for_alias_announced().await;

      loop {
        if instance.is_finished().await {
          break;
        }

        tokio::select! {
          biased;
          _ = instance.receive() => {
            continue;
          }
          // TODO: implement max timeout here
          // 5 second timeout to check if the subscription is still valid
          _ = tokio::time::sleep(tokio::time::Duration::from_millis(5000)) => {
            continue;
          }
        }
      }
    });

    sub
  }

  pub async fn is_finished(&self) -> bool {
    self.finished.load(Ordering::Relaxed)
  }

  pub async fn is_forwarding(&self) -> bool {
    let state = self.subscription_state.read().await;
    state.forward
  }

  /// Number of data streams opened for this subscription (PUBLISH_DONE Stream
  /// Count), including subgroups that carried no objects.
  pub fn relay_track_id(&self) -> u64 {
    self.relay_track_id
  }

  /// Payload bytes fanned out to this subscription and not yet taken by its task.
  pub fn queued_bytes(&self) -> u64 {
    self.queue.bytes.load(Ordering::Relaxed) as u64
  }

  pub fn opened_stream_count(&self) -> u64 {
    self.opened_stream_count.load(Ordering::Relaxed)
  }

  pub fn subscriber(&self) -> Arc<MOQTClient> {
    self.subscriber.clone()
  }

  /// The Track Properties to put in the SUBSCRIBE_OK: `properties`, or none when the
  /// SUBSCRIBE set INCLUDE_PROPERTIES to 0.
  pub async fn properties_for_ok(&self, properties: Vec<TrackProperty>) -> Vec<TrackProperty> {
    let state = self.subscription_state.read().await;
    if include_properties(&state.subscribe_parameters) {
      properties
    } else {
      Vec::new()
    }
  }

  // This method updates the subscribe message with the new request update
  // Returns Ok if the update is successful
  // Returns error if the update is invalid
  pub async fn update_subscription(&self, request_update: RequestUpdate) -> Result<()> {
    let (forward_becoming_true, forward_becoming_false) = {
      let mut state = self.subscription_state.write().await;

      // An update changes the filter only when it carries one. A relative filter
      // is resolved against the track by the caller, which knows its Largest
      // Object.
      if let Some(MessageParameter::LocationFilter { filter }) = request_update
        .parameters
        .iter()
        .find(|p| matches!(p, MessageParameter::LocationFilter { .. }))
      {
        state.set_location_filter(filter.clone(), None);
      }

      // Update explicit subscription state fields if they are present in the parameters.
      // Track which way forward transitioned: false to true flushes pending_header
      // below, true to false stops anything still filling.
      let mut transition = false;
      let mut forward_becoming_false = false;
      for param in &request_update.parameters {
        match param {
          MessageParameter::SubscriberPriority { priority } => {
            state.subscriber_priority = *priority;
          }
          MessageParameter::Forward { forward } => {
            if *forward && !state.forward {
              transition = true;
            }
            if !*forward && state.forward {
              forward_becoming_false = true;
            }
            state.forward = *forward;
          }
          _ => {}
        }
      }

      // Update parameters. If a parameter included in SUBSCRIBE is not present in
      // REQUEST_UPDATE, its value remains unchanged. There is no mechanism
      // to remove a parameter from a request.
      apply_message_parameter_update(&mut state.subscribe_parameters, request_update.parameters);
      // The handler refused the update if the filters it leaves could not be applied.
      state.range_filter =
        CombinedFilter::for_objects(&state.subscribe_parameters).unwrap_or_default();

      info!(
        "update_subscription | new subscription state for relay_track_id={}: {:?}",
        self.relay_track_id, state
      );

      (transition, forward_becoming_false)
      // write lock on subscription_state is dropped here
    };

    // Forward State 0 ends any fill in progress along with live delivery.
    if forward_becoming_false {
      self.stop_fill_streams(FetchStop::Cancelled).await;
    }

    // If forward just became true, open the stream for the current mid-group header
    // that was cached while forward=false.
    if forward_becoming_true {
      let pending = self.pending_header.lock().await.take();
      if let Some((pending_stream_id, pending_header_info)) = pending {
        info!(
          "update_subscription | forward became true, opening pending stream {} for subscriber={} relay_track_id={}",
          pending_stream_id, self.client_connection_id, self.relay_track_id
        );
        // The cached header came with an object that was not forwarded, so this
        // stream starts after the subgroup's first object.
        if self
          .handle_header(for_mid_subgroup_stream(pending_header_info))
          .await
          .is_ok()
        {
          self
            .send_stream_last_object_ids
            .write()
            .await
            .insert(pending_stream_id, None);
        }
      }
    }

    Ok(())
  }

  pub async fn finish(&self) {
    if self
      .finished
      .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
      .is_err()
    {
      return;
    }

    info!(
      "Finishing subscription for subscriber={} relay_track_id={}",
      self.client_connection_id, self.relay_track_id
    );

    info!(
      "Subscription finished for subscriber={} relay_track_id={}",
      self.client_connection_id, self.relay_track_id
    );

    // Close all send streams asynchronously to avoid blocking subscription cleanup
    let stream_ids = {
      let mut send_stream_last_object_ids = self.send_stream_last_object_ids.write().await;
      let ids = send_stream_last_object_ids
        .keys()
        .map(|stream_id| self.new_send_stream_key(stream_id))
        .collect::<Vec<_>>();
      send_stream_last_object_ids.clear();
      ids
    };

    if !stream_ids.is_empty() {
      let subscriber = self.subscriber.clone();
      let connection_id = self.client_connection_id;
      let relay_track_id = self.relay_track_id;

      // Spawn background task for graceful stream cleanup
      tokio::spawn(async move {
        info!(
          "Starting background cleanup of {} streams for subscriber={} relay_track_id={}",
          stream_ids.len(),
          connection_id,
          relay_track_id
        );

        for stream_id in stream_ids.iter() {
          let res = subscriber.close_stream(stream_id).await;
          if let Err(e) = res {
            warn!(
              "Background stream cleanup error for subscriber={} stream_id={} relay_track_id={} error: {:?}",
              connection_id, stream_id, relay_track_id, e
            );
          } else if let Ok(closed) = res {
            if closed {
              trace!(
                "Background stream cleanup successful for subscriber={} stream_id={} relay_track_id={}",
                connection_id, stream_id, relay_track_id
              );
            } else {
              trace!(
                "Background stream cleanup: stream not found for subscriber={} stream_id={} relay_track_id={}",
                connection_id, stream_id, relay_track_id
              );
            }
          }
        }

        info!(
          "Background cleanup completed for subscriber={} relay_track_id={} ({} streams)",
          connection_id,
          relay_track_id,
          stream_ids.len()
        );
      });
    }
  }

  async fn receive(&mut self) {
    trace!(
      "Receiving for subscriber: {} track: {}",
      self.client_connection_id, self.relay_track_id
    );

    let mut event_rx_guard = self.event_rx.lock().await;
    let (recv_result, backlog) = {
      let Some(ref mut rx) = *event_rx_guard else {
        return;
      };
      let event = rx.recv().await;
      let backlog = rx.len();
      (event, backlog)
    };
    if let Some((_, event)) = &recv_result {
      self
        .queue
        .bytes
        .fetch_sub(event.queued_bytes(), Ordering::Relaxed);
    }

    trace!(
      "Received event for subscriber={} relay_track_id={}: {:?}",
      self.client_connection_id, self.relay_track_id, recv_result
    );

    // Slow-subscriber shedding: once the queued backlog exceeds the limit, the
    // subscriber can't keep up, so reset its data streams with TOO_FAR_BEHIND.
    let max_lag = self.config.max_subscriber_lag;
    if max_lag > 0 && backlog as u64 > max_lag && !self.finished.load(Ordering::Relaxed) {
      warn!(
        "Subscriber {} too far behind on relay_track_id={} ({} queued > {}); resetting data streams (TOO_FAR_BEHIND)",
        self.client_connection_id, self.relay_track_id, backlog, max_lag
      );
      event_rx_guard.take();
      drop(event_rx_guard);
      self
        .reset_data_streams(StreamResetCode::TooFarBehind.to_u64())
        .await;
      self.finish().await;
      return;
    }

    match recv_result {
      Some((queued_at, event)) if !self.finished.load(Ordering::Relaxed) => {
        drop(event_rx_guard);
        self.handle_track_event(queued_at, event).await;
      }
      Some(_) => {
        event_rx_guard.take();
      }
      None => {
        info!(
          "Event receiver closed for subscriber={} relay_track_id={}, finishing subscription",
          self.client_connection_id, self.relay_track_id
        );
        self.finish().await;
        event_rx_guard.take();
        drop(event_rx_guard);
      }
    }
  }

  /// Ends the subscription because the subscriber cancelled it.
  ///
  /// A cancelled subscription's streams are reset, not finished. A finish is a FIN,
  /// which asks the peer to take delivery of everything already written — data the
  /// subscriber has just said it no longer wants. Its fills stop too, as a cancelled
  /// fetch does.
  pub async fn cancel(&self) {
    info!(
      "Cancelling subscription for subscriber={} relay_track_id={}",
      self.client_connection_id, self.relay_track_id
    );
    self
      .reset_data_streams(StreamResetCode::Cancelled.to_u64())
      .await;
    self.stop_fill_streams(FetchStop::Cancelled).await;
    self.finish().await;
  }

  /// Send PublishDone message to this subscriber
  pub async fn send_publish_done(
    &self,
    status_code: PublishDoneStatusCode,
    reason: &str,
  ) -> Result<(), anyhow::Error> {
    let reason_phrase = ReasonPhrase::try_new(reason.to_string())
      .map_err(|e| anyhow::anyhow!("Failed to create reason phrase: {:?}", e))?;

    let publish_done = PublishDone::new(
      status_code,
      self.opened_stream_count.load(Ordering::Relaxed),
      reason_phrase,
    );

    // PUBLISH_DONE ends a subscription, so it belongs on that subscription's own
    // request stream rather than the control stream.
    if !self
      .subscriber
      .send_response(
        self.request_id,
        ControlMessage::PublishDone(Box::new(publish_done)),
      )
      .await
    {
      warn!(
        "No request stream for subscriber={} request_id={}; PUBLISH_DONE dropped",
        self.client_connection_id, self.request_id
      );
      return Ok(());
    }

    info!(
      "Sent PublishDone to subscriber={} relay_track_id={} for request_id={} stream_count={}",
      self.client_connection_id,
      self.relay_track_id,
      self.request_id,
      self.opened_stream_count.load(Ordering::Relaxed)
    );

    Ok(())
  }
}
