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

use crate::server::client::MOQTClient;
use crate::server::client::switch_context::{SwitchPlan, SwitchStatus};
use crate::server::config::AppConfig;
use crate::server::message_handlers::fetch_handler::FetchStop;
use crate::server::object_logger::ObjectLogger;
use crate::server::stream_id::StreamId;
use crate::server::track::ActiveSubgroupHeaderMap;
use crate::server::track::TrackEvent;
use crate::server::track_cache::TrackCache;
use crate::server::utils;
use anyhow::Result;
use bytes::Bytes;
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::FilterType;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::constant::SwitchMode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish::Publish;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::control::request_update::RequestUpdate;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::data::object::Object;
use moqtail::model::error::StreamResetCode;
use moqtail::model::parameter::message_parameter::{
  MessageParameter, apply_message_parameter_update,
};
use moqtail::transport::connection::TransportSendStream;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::watch;
use tracing::{debug, error, info, trace, warn};

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

#[derive(Debug, Clone)]
pub struct SubscriptionState {
  pub subscriber_priority: u8,
  pub group_order: GroupOrder,
  pub forward: bool,
  pub filter_type: FilterType,
  pub start_location: Option<Location>,
  pub end_group: u64,
  /// How many groups back from the Largest Object a RelativeStartFill starts.
  pub relative_previous: Option<u64>,
  /// Set on a subscription a soft switch is draining: it keeps delivering up to
  /// `end_group` rather than stopping where it stands.
  pub soft_suspended: bool,
  /// Whether reaching the end of that drain also sends PUBLISH_DONE.
  pub publish_done_at_end: bool,
  pub subscribe_parameters: Vec<MessageParameter>,
  pub last_sent_max_location: Option<Location>,
  pub last_received_object_location: Option<Location>,
}

impl SubscriptionState {
  pub fn update_last_sent_max_location(&mut self, location: Location) {
    match &self.last_sent_max_location {
      Some(current_max) => {
        if location > *current_max {
          self.last_sent_max_location = Some(location);
        }
      }
      None => {
        self.last_sent_max_location = Some(location);
      }
    }
  }

  pub fn update_last_received_object_location(&mut self, location: Location) {
    match &self.last_received_object_location {
      Some(current_max) => {
        if location > *current_max {
          self.last_received_object_location = Some(location);
        }
      }
      None => {
        self.last_received_object_location = Some(location);
      }
    }
  }
}

impl From<SubscriptionOrigin> for SubscriptionState {
  fn from(origin: SubscriptionOrigin) -> Self {
    match origin {
      SubscriptionOrigin::Subscribe(subscribe) => {
        let subscriber_priority = subscribe
          .subscribe_parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::SubscriberPriority { priority } = p {
              Some(*priority)
            } else {
              None
            }
          })
          .unwrap_or(128);

        let group_order = subscribe
          .subscribe_parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::GroupOrder { order } = p {
              Some(*order)
            } else {
              None
            }
          })
          .unwrap_or(GroupOrder::Ascending);

        let forward = subscribe
          .subscribe_parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::Forward { forward } = p {
              Some(*forward)
            } else {
              None
            }
          })
          .unwrap_or(true);

        let (filter_type, start_location, end_group, relative_previous) = subscribe
          .subscribe_parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::SubscriptionFilter {
              filter_type,
              start_location,
              end_group,
              relative_previous,
            } = p
            {
              Some((
                *filter_type,
                start_location.clone(),
                end_group.unwrap_or(0),
                *relative_previous,
              ))
            } else {
              None
            }
          })
          .unwrap_or((FilterType::LatestObject, None, 0, None));

        Self {
          subscriber_priority,
          group_order,
          forward,
          filter_type,
          start_location,
          end_group,
          relative_previous,
          soft_suspended: false,
          publish_done_at_end: false,
          subscribe_parameters: subscribe.subscribe_parameters,
          last_sent_max_location: None,
          last_received_object_location: None,
        }
      }
      SubscriptionOrigin::Publish(publish) => {
        let subscriber_priority = publish
          .parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::SubscriberPriority { priority } = p {
              Some(*priority)
            } else {
              None
            }
          })
          .unwrap_or(128);

        let group_order = publish
          .parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::GroupOrder { order } = p {
              Some(*order)
            } else {
              None
            }
          })
          .unwrap_or(GroupOrder::Ascending);

        let forward = publish
          .parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::Forward { forward } = p {
              Some(*forward)
            } else {
              None
            }
          })
          .unwrap_or(true);

        let (filter_type, start_location, end_group, relative_previous) = publish
          .parameters
          .iter()
          .find_map(|p| {
            if let MessageParameter::SubscriptionFilter {
              filter_type,
              start_location,
              end_group,
              relative_previous,
            } = p
            {
              Some((
                *filter_type,
                start_location.clone(),
                end_group.unwrap_or(0),
                *relative_previous,
              ))
            } else {
              None
            }
          })
          .unwrap_or((FilterType::LatestObject, None, 0, None));

        Self {
          subscriber_priority,
          group_order,
          forward,
          filter_type,
          start_location,
          end_group,
          relative_previous,
          soft_suspended: false,
          publish_done_at_end: false,
          subscribe_parameters: publish.parameters,
          last_sent_max_location: None,
          last_received_object_location: None,
        }
      }
    }
  }
}

/// The publisher priority assumed where a stream carries no single Object's own.
pub(crate) const DEFAULT_PUBLISHER_PRIORITY: u8 = 128;

/// Compute QUIC stream priority from MOQT scheduling parameters.
///
/// The i32 space is divided into 65536 bands (one per sub_prio × pub_prio pair).
/// Within each band, group_id determines relative position according to group_order:
///   Ascending / Original – lower group_id = higher priority (counts down from band_max)
///   Descending            – higher group_id = higher priority (counts up from band_min)
pub(crate) fn compute_stream_priority(
  sub_prio: u8,
  pub_prio: u8,
  group_order: GroupOrder,
  group_id: u64,
) -> i32 {
  const BAND_SIZE: i64 = 65536;
  let priority_index = (255 - sub_prio as i64) * 256 + (255 - pub_prio as i64);
  let band_min = i32::MIN as i64 + priority_index * BAND_SIZE;
  let group_slot = (group_id % BAND_SIZE as u64) as i64;
  match group_order {
    GroupOrder::Ascending | GroupOrder::Original => (band_min + BAND_SIZE - 1 - group_slot) as i32,
    GroupOrder::Descending => (band_min + group_slot) as i32,
  }
}

#[derive(Debug, Clone)]
pub struct Subscription {
  pub request_id: u64,
  relay_track_id: u64,
  pub full_track_name: FullTrackName,
  pub subscription_state: Arc<RwLock<SubscriptionState>>,
  subscriber: Arc<MOQTClient>,
  event_rx: Arc<Mutex<Option<UnboundedReceiver<TrackEvent>>>>,
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
  check_switch_context_on_next_object: Arc<AtomicBool>,
  /// Subgroup header cached while forward=false. Cleared when forward becomes true (stream opened)
  /// or when a new group starts (old group ended without forward ever becoming true).
  pending_header: Arc<Mutex<Option<(StreamId, HeaderInfo)>>>,
  /// Shared map of open publisher subgroup streams and their original subgroup header.
  /// Used to open a QUIC send stream for a new mid-group subscriber with the exact
  /// original header rather than a synthesized one.
  active_subgroup_headers: ActiveSubgroupHeaderMap,
  /// Set once the control message carrying this subscriber's track alias has gone out
  /// (SUBSCRIBE_OK, or PUBLISH when the relay pushes). Data streams carry only the
  /// alias, so a subscriber that gets one first cannot place it and drops the objects.
  /// Forwarding waits for this, and the queued Objects follow in order.
  alias_announced: Arc<AtomicBool>,
  alias_announced_notify: Arc<Notify>,
  /// Fill fetch streams still delivering, keyed by the request that asked for the
  /// fill. Held so Forward State 0 and cancellation can stop them.
  fill_streams: Arc<RwLock<HashMap<u64, watch::Sender<FetchStop>>>>,
}

#[allow(clippy::too_many_arguments)]
impl Subscription {
  fn create_instance(
    relay_track_id: u64,
    full_track_name: FullTrackName,
    request_id: u64,
    origin_message: SubscriptionOrigin,
    subscriber: Arc<MOQTClient>,
    event_rx: Arc<Mutex<Option<UnboundedReceiver<TrackEvent>>>>,
    cache: TrackCache,
    client_connection_id: usize,
    log_folder: String,
    config: &'static AppConfig,
    active_subgroup_headers: ActiveSubgroupHeaderMap,
  ) -> Self {
    Self {
      relay_track_id,
      full_track_name,
      request_id,
      subscription_state: Arc::new(RwLock::new(origin_message.into())),
      subscriber,
      event_rx,
      send_stream_last_object_ids: Arc::new(RwLock::new(HashMap::new())),
      opened_stream_count: Arc::new(AtomicU64::new(0)),
      finished: Arc::new(AtomicBool::new(false)),
      cache,
      client_connection_id,
      object_logger: ObjectLogger::new(log_folder),
      config,
      check_switch_context_on_next_object: Arc::new(AtomicBool::new(false)),
      pending_header: Arc::new(Mutex::new(None)),
      active_subgroup_headers,
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
    full_track_name: FullTrackName,
    origin_message: SubscriptionOrigin,
    subscriber: Arc<MOQTClient>,
    event_rx: UnboundedReceiver<TrackEvent>,
    cache: TrackCache,
    client_connection_id: usize,
    log_folder: String,
    config: &'static AppConfig,
    active_subgroup_headers: ActiveSubgroupHeaderMap,
  ) -> Self {
    let event_rx = Arc::new(Mutex::new(Some(event_rx)));
    let sub = Self::create_instance(
      relay_track_id,
      full_track_name,
      origin_message.request_id(),
      origin_message,
      subscriber,
      event_rx,
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
  pub fn opened_stream_count(&self) -> u64 {
    self.opened_stream_count.load(Ordering::Relaxed)
  }

  pub fn subscriber(&self) -> Arc<MOQTClient> {
    self.subscriber.clone()
  }

  // This method updates the subscribe message with the new request update
  // Returns Ok if the update is successful
  // Returns error if the update is invalid
  pub async fn update_subscription(&self, request_update: RequestUpdate) -> Result<()> {
    let (forward_becoming_true, forward_becoming_false) = {
      let mut state = self.subscription_state.write().await;

      // Extract filter_type, start_location and end_group from SubscriptionFilter parameter
      let (new_filter_type, new_start_location, new_end_group) = request_update
        .parameters
        .iter()
        .find_map(|p| {
          if let MessageParameter::SubscriptionFilter {
            filter_type,
            start_location,
            end_group,
            ..
          } = p
          {
            Some((Some(*filter_type), start_location.clone(), *end_group))
          } else {
            None
          }
        })
        .unwrap_or((None, None, None));

      if let Some(ref new_loc) = new_start_location {
        state.start_location = Some(new_loc.clone());
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

      if let Some(ft) = new_filter_type {
        state.filter_type = ft;
      }
      if let Some(eg) = new_end_group {
        state.end_group = eg;
      }

      // Update parameters. If a parameter included in SUBSCRIBE is not present in
      // REQUEST_UPDATE, its value remains unchanged. There is no mechanism
      // to remove a parameter from a request.
      apply_message_parameter_update(&mut state.subscribe_parameters, request_update.parameters);

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
        if self.handle_header(pending_header_info).await.is_ok() {
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
        .cloned()
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

  // Notify the subscription to check the switch context on the next object
  pub async fn notify_switch(&self) {
    info!(
      "Notifying subscription to check switch context on next object for subscriber={} relay_track_id={}",
      self.client_connection_id, self.relay_track_id
    );
    self
      .check_switch_context_on_next_object
      .store(true, std::sync::atomic::Ordering::Relaxed);
  }

  /// Stops the subscription a switch suspends, at the moment the activating one is
  /// ready to take over.
  ///
  /// A hard switch cuts: Forward State 0 and every stream it still has open is
  /// reset, fill fetch streams included. Objects already in flight can still
  /// arrive. PUBLISH_DONE follows only if the switch asked for it.
  ///
  /// A soft switch instead lets it run out: its End Group becomes the group before
  /// the activating subscription's Start Group, so the two are contiguous, and its
  /// streams — fill fetch streams included — are left to finish. There is no group
  /// before group 0, so a Start Group of 0 leaves nothing to drain and cuts.
  async fn stop_suspending_subscription(&self, plan: &SwitchPlan, start_group: u64) {
    let Some(weak) = self
      .subscriber
      .subscriptions
      .get_subscription(&plan.suspending)
      .await
    else {
      warn!(
        "switch: no subscription left to suspend for {:?} on subscriber {}",
        plan.suspending, self.client_connection_id
      );
      return;
    };
    let Some(suspending) = weak.upgrade() else {
      return;
    };
    let suspending = suspending.read().await;

    info!(
      "switch: suspending {:?} for subscriber {} ({:?})",
      plan.suspending, self.client_connection_id, plan.mode
    );

    // A soft switch resets nothing here: the subscription keeps its streams and any
    // fill still running, and delivers up to the boundary. `finish_soft_drain`
    // closes them once it gets there.
    if plan.mode == SwitchMode::Soft && start_group > 0 {
      let mut state = suspending.subscription_state.write().await;
      state.soft_suspended = true;
      state.end_group = start_group - 1;
      state.publish_done_at_end = plan.publish_done;
      info!(
        "switch: draining {:?} for subscriber {} up to group {}",
        plan.suspending, self.client_connection_id, state.end_group
      );
      return;
    }

    suspending.subscription_state.write().await.forward = false;
    suspending.stop_fill_streams(FetchStop::Cancelled).await;
    suspending
      .reset_data_streams(StreamResetCode::Cancelled.to_u64())
      .await;

    if plan.publish_done
      && let Err(e) = suspending
        .send_publish_done(
          PublishDoneStatusCode::SubscriptionEnded,
          "switched away from",
        )
        .await
    {
      error!(
        "switch: failed to send PUBLISH_DONE to the suspended subscription: {:?}",
        e
      );
    }
  }

  /// Ends a soft switch's drain: the suspending subscription has delivered
  /// everything up to its boundary, so its streams are closed and PUBLISH_DONE
  /// goes out if the switch asked for it.
  async fn finish_soft_drain(&self) {
    {
      let mut state = self.subscription_state.write().await;
      if !state.soft_suspended {
        return;
      }
      state.soft_suspended = false;
      state.forward = false;
    }

    let publish_done = {
      let mut state = self.subscription_state.write().await;
      let wanted = state.publish_done_at_end;
      state.publish_done_at_end = false;
      wanted
    };

    info!(
      "switch: drain complete for subscriber={} relay_track_id={}",
      self.client_connection_id, self.relay_track_id
    );

    if publish_done {
      if let Err(e) = self
        .send_publish_done(
          PublishDoneStatusCode::SubscriptionEnded,
          "switched away from",
        )
        .await
      {
        error!(
          "switch: failed to send PUBLISH_DONE after the drain: {:?}",
          e
        );
      }
      self.finish().await;
    }
  }

  async fn check_switch_context(&self, object_location: &Location) -> bool {
    // if the object is after the end group, finish the subscription
    let status = self
      .subscriber
      .switch_context
      .get_switch_status(&self.full_track_name)
      .await;

    if status.is_none() {
      // not in a switch context, always forward
      return true;
    }

    let status = status.unwrap();

    match status {
      SwitchStatus::Next => {
        // check whether the group id of this track
        // is equal to or greater than the one of
        // the switch context's current track
        // if so, set this track as current
        let mut switch_at_next_group = false;
        let mut new_start_location = None;

        // The subscriber can ask to switch at a future group. Forwarding begins at the
        // group after this object, so until that reaches the requested start group the
        // switch is not due and the track being switched away from keeps delivering.
        let requested_start = { self.subscription_state.read().await.start_location.clone() };
        let before_requested_start = requested_start
          .as_ref()
          .is_some_and(|start| object_location.group + 1 < start.group);

        if before_requested_start {
          switch_at_next_group = false;
        } else if let Some(current_track_name) = self.subscriber.switch_context.get_current().await
        {
          let current_subscription_opt = self
            .subscriber
            .subscriptions
            .get_subscription(&current_track_name)
            .await;

          if let Some(current_subscription) = current_subscription_opt
            && let Some(current_subscription) = current_subscription.upgrade()
          {
            let current_subscription = current_subscription.read().await;
            let current_state = current_subscription.subscription_state.read().await;
            let last_sent_max_location = current_state.last_sent_max_location.clone();

            if let Some(loc) = last_sent_max_location {
              switch_at_next_group = object_location.group >= loc.group;
              let mut loc_clone = loc.clone();
              // We modify the start location of the subscriptions to manage the switch context
              // that's why we always set to next in a switch because this may not be the first switch
              loc_clone.group += 1; // switch at the next group after the last sent max location of the current track
              loc_clone.object = 0; // reset object id to 0 to read from the start of the group
              new_start_location = Some(loc_clone);
            } else {
              // if there is no last sent location, we can switch
              switch_at_next_group = true;
            }
          }
        } else {
          // no current track, we can switch
          switch_at_next_group = true;
        }

        if switch_at_next_group {
          // set this track as current
          let subscriber = self.subscriber.clone();
          let full_track_name = self.full_track_name.clone();

          // the following method also sets the current active track's status to None if any
          info!(
            "check_switch_context: Setting track to Current for subscriber={} relay_track_id={} object location group: {}",
            self.client_connection_id, self.relay_track_id, object_location.group
          );
          subscriber
            .switch_context
            .add_or_update_switch_item(full_track_name.clone(), SwitchStatus::Current)
            .await;

          // set forward to true and set start group the next group
          let mut state = self.subscription_state.write().await;
          state.forward = true;

          // We only get here once:
          //   suspend track's subscription's object group + 1
          //     >= resume track's requested start group (comes in the subscribe message)
          // so the start group set below is the requested one or later. Say the
          // subscriber asks to start at group 20 and the suspend track is sending
          // group 19: the resume track starts at group 20.
          if new_start_location.is_some() {
            state.start_location = new_start_location;
          } else {
            state.start_location = Some(Location {
              object: 0,
              group: object_location.group + 1,
            });
          }

          state.end_group = 0; // remove end group limit

          let start_group = state.start_location.as_ref().unwrap().group;
          info!(
            "check_switch_context: Will forward objects for subscriber={} relay_track_id={} starting from group: {}",
            self.client_connection_id, self.relay_track_id, start_group
          );
          drop(state);

          // The activating subscription is ready to publish from the Start Group,
          // so this is the moment the suspending one stops.
          if let Some(plan) = subscriber.switch_context.take_plan(&full_track_name).await {
            self.stop_suspending_subscription(&plan, start_group).await;
          }
        } else {
          // Do not forward objects for Next status until switch condition is met
          // set forward to false if it is true
          if self.is_forwarding().await {
            info!(
              "check_switch_context: Setting forward to false for Next track for subscriber={} relay_track_id={} object location group: {}",
              self.client_connection_id, self.relay_track_id, object_location.group
            );
            self.subscription_state.write().await.forward = false;
          }
        }
        // even if the switch_at_next_group is true,
        // we return false here to wait for the next group to switch
        false
      }
      SwitchStatus::Current => true,
      SwitchStatus::None => {
        // A soft switch already told this subscription where to stop, and it keeps
        // delivering until it gets there.
        if self.subscription_state.read().await.soft_suspended {
          return true;
        }
        // set forward to false if it is true
        if self.is_forwarding().await {
          info!(
            "check_switch_context: Setting end group to {} for None track for subscriber={} relay_track_id={}",
            object_location.group, self.client_connection_id, self.relay_track_id
          );
          let mut state = self.subscription_state.write().await;
          state.forward = false;
          state.end_group = object_location.group;
        }

        false
      }
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
      Some(event) if !self.finished.load(Ordering::Relaxed) => {
        drop(event_rx_guard);
        self.handle_track_event(event).await;
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

  /// Reset every data stream this subscription has opened with the given code.
  /// Resets every open data stream, draining the map so `finish` does not then try to
  /// close a stream that has already been reset.
  async fn reset_data_streams(&self, code: u64) {
    let stream_ids: Vec<StreamId> = {
      let mut map = self.send_stream_last_object_ids.write().await;
      map.drain().map(|(stream_id, _)| stream_id).collect()
    };
    for stream_id in stream_ids {
      self.subscriber.reset_stream(&stream_id, code).await;
    }
  }

  /// Ends the subscription because the subscriber cancelled it.
  ///
  /// A cancelled subscription's streams are reset, not finished. A finish is a FIN,
  /// which asks the peer to take delivery of everything already written — data the
  /// subscriber has just said it no longer wants.
  pub async fn cancel(&self) {
    info!(
      "Cancelling subscription for subscriber={} relay_track_id={}",
      self.client_connection_id, self.relay_track_id
    );
    self
      .reset_data_streams(StreamResetCode::Cancelled.to_u64())
      .await;
    self.finish().await;
  }

  async fn handle_track_event(&self, event: TrackEvent) {
    trace!(
      "Event received for subscriber={} relay_track_id={} event: {:?}",
      self.client_connection_id, self.relay_track_id, event
    );
    match event {
      TrackEvent::SubgroupObject {
        mut object,
        stream_id,
        header_info,
      } => {
        object.track_alias = self.relay_track_id;
        // update last received object location
        {
          let mut state = self.subscription_state.write().await;
          state.update_last_received_object_location(object.location.clone());
        }

        // Check switch context state if needed
        // Whether when a new header is received or when notified about a switch context change
        let check_switch = self
          .check_switch_context_on_next_object
          .load(std::sync::atomic::Ordering::Relaxed);
        if header_info.is_some() || check_switch {
          if check_switch {
            self
              .check_switch_context_on_next_object
              .store(false, std::sync::atomic::Ordering::Relaxed);
          }
          // Check whether this track is in a switch context and update forward state
          if !self.check_switch_context(&object.location).await {
            // if this returns false, do not start the stream
            info!(
              "Not forwarding object for subscriber={} relay_track_id={} due to switch context state",
              self.client_connection_id, self.relay_track_id
            );
            return;
          }
        }

        let object_received_time = utils::passed_time_since_start();

        {
          let state = self.subscription_state.read().await;
          if let Some(start) = &state.start_location
            && object.location < *start
          {
            trace!(
              "Object before start location for subscriber={} relay_track_id={} object location: {:?} start location: {:?}",
              self.client_connection_id, self.relay_track_id, object.location, start
            );
            return;
          }

          if state.end_group > 0 && object.location.group > state.end_group {
            trace!(
              "Object beyond end group for subscriber={} relay_track_id={} object location: {:?} end group: {}",
              self.client_connection_id, self.relay_track_id, object.location, state.end_group
            );
            let drained = state.soft_suspended;
            drop(state);
            // An object past the boundary is how a drain ends: the group before it
            // has nothing more to give.
            if drained {
              self.finish_soft_drain().await;
            }
            return;
          }

          if !state.forward {
            // Cache the subgroup header so we can open the stream immediately
            // if forward transitions to true mid-group (sub-update-forward method).
            if let Some(ref header) = header_info {
              let mut pending = self.pending_header.lock().await;
              *pending = Some((stream_id.clone(), header.clone()));
            }
            return;
          }
        }

        // Entering forward=true: clear any stale pending header (group boundary case).
        // If forward was already true, pending_header is None and this is a no-op.
        {
          let mut pending = self.pending_header.lock().await;
          pending.take();
        }

        // Handle header info if this is the first object
        let send_stream = if let Some(header) = header_info {
          if let HeaderInfo::Subgroup { header: _ } = header {
            debug!(
              "Creating stream - subscriber={} relay_track_id={} now={} received time={} object: {:?} header: {:?}",
              self.client_connection_id,
              self.relay_track_id,
              utils::passed_time_since_start(),
              object_received_time,
              object.location,
              header
            );
            if let Ok((stream_id, send_stream)) = self.handle_header(header.clone()).await {
              {
                let mut send_stream_last_object_ids =
                  self.send_stream_last_object_ids.write().await;
                send_stream_last_object_ids.insert(stream_id.clone(), None);
              }
              debug!(
                "Stream created - subscriber={} stream_id={} relay_track_id={} now={} received time={} object: {:?}",
                self.client_connection_id,
                stream_id,
                self.relay_track_id,
                utils::passed_time_since_start(),
                object_received_time,
                object.location
              );
              Some(send_stream)
            } else {
              // TODO: maybe log error here?
              None
            }
          } else {
            error!(
              "Received Object event with non-subgroup header: {:?}",
              header
            );
            None
          }
        } else {
          match self.subscriber.get_stream(&stream_id).await {
            Some(s) => Some(s),
            None => {
              // New subscriber joined mid-subgroup. Look up the original header
              // from the track-level cache and open a QUIC send stream with it.
              let cached = self
                .active_subgroup_headers
                .read()
                .await
                .get(&stream_id)
                .cloned();
              if let Some(h) = cached {
                trace!(
                  "mid-subgroup join: opening stream from cached header for subscriber={} relay_track_id={} stream_id={}",
                  self.client_connection_id, self.relay_track_id, stream_id
                );
                self
                  .handle_header(h)
                  .await
                  .ok()
                  .map(|(_, send_stream)| send_stream)
              } else {
                None
              }
            }
          }
        };

        if let Some(send_stream) = send_stream {
          // Get the previous object ID for this stream
          let previous_object_id = {
            let send_stream_last_object_ids = self.send_stream_last_object_ids.read().await;
            send_stream_last_object_ids
              .get(&stream_id)
              .cloned()
              .flatten()
          };

          trace!(
            "Received Object event: subscriber={} stream_id={} relay_track_id={} previous_object_id: {:?} object: {:?} now={} received time={}",
            self.client_connection_id,
            stream_id,
            self.relay_track_id,
            previous_object_id,
            object,
            utils::passed_time_since_start(),
            object_received_time
          );

          // Log object properties with send status if enabled
          let write_result = self
            .handle_object(
              object.clone(),
              previous_object_id,
              &stream_id,
              send_stream.clone(),
            )
            .await;
          let send_status = write_result.is_ok();

          // Update the last object ID for this stream if successful
          if send_status {
            let mut send_stream_last_object_ids = self.send_stream_last_object_ids.write().await;
            send_stream_last_object_ids.insert(stream_id.clone(), Some(object.location.object));
            drop(send_stream_last_object_ids); // Release the lock immediately

            // Update last sent max location
            self
              .subscription_state
              .write()
              .await
              .update_last_sent_max_location(object.location.clone());
          }

          if self.config.enable_object_logging {
            self
              .object_logger
              .log_subscription_object(
                self.relay_track_id,
                self.client_connection_id,
                &object,
                send_status,
                object_received_time,
              )
              .await;
          }
        } else {
          error!(
            "Received Object event without a valid send stream for subscriber={} stream_id={} relay_track_id={} object: {:?} now={} received time={}",
            self.client_connection_id,
            stream_id,
            self.relay_track_id,
            object.location,
            utils::passed_time_since_start(),
            object_received_time
          );
        }
      }
      TrackEvent::Datagram { object } => {
        // Handle datagram - serialize full MOQT datagram format
        // Must include type, track_alias, group_id, object_id, publisher_priority, and payload

        let location = Location::new(object.group_id, object.object_id);
        {
          let mut state = self.subscription_state.write().await;
          state.update_last_received_object_location(location.clone());
        }

        // A datagram is subject to the same filter and forward state as any other
        // object. There is no stream to hold open, so nothing is cached for a later
        // forward transition: a datagram missed while paused is simply missed.
        {
          let state = self.subscription_state.read().await;
          if let Some(start) = &state.start_location
            && location < *start
          {
            trace!(
              "Datagram before start location for subscriber={} relay_track_id={} object location: {:?} start location: {:?}",
              self.client_connection_id, self.relay_track_id, location, start
            );
            return;
          }

          if state.end_group > 0 && location.group > state.end_group {
            trace!(
              "Datagram beyond end group for subscriber={} relay_track_id={} object location: {:?} end group: {}",
              self.client_connection_id, self.relay_track_id, location, state.end_group
            );
            return;
          }

          if !state.forward {
            trace!(
              "Not forwarding datagram for subscriber={} relay_track_id={}: forward state is 0",
              self.client_connection_id, self.relay_track_id
            );
            return;
          }
        }

        let mut norm_object = object.clone();
        norm_object.track_alias = self.relay_track_id;

        match norm_object.serialize() {
          Ok(serialized_bytes) => {
            if let Err(e) = self
              .subscriber
              .write_datagram_object(serialized_bytes)
              .await
            {
              error!("Failed to write datagram: {:?}", e);
            }
          }
          Err(e) => {
            error!("Failed to serialize datagram: {:?}", e);
          }
        }
      }
      TrackEvent::StreamClosed { stream_id } => {
        debug!(
          "Received StreamClosed event: subscriber={} stream_id={} relay_track_id={}",
          self.client_connection_id, stream_id, self.relay_track_id
        );
        let _ = self.handle_stream_closed(&stream_id).await;
      }
      TrackEvent::PublisherDisconnected {
        status_code,
        reason,
      } => {
        info!(
          "Received PublisherDisconnected event: subscriber={}, status={:?} reason={} relay_track_id={}",
          self.client_connection_id, status_code, reason, self.relay_track_id
        );

        // Send PublishDone message and finish the subscription
        if let Err(e) = self.send_publish_done(status_code, &reason).await {
          error!(
            "Failed to send PublishDone for publisher disconnect: subscriber={} relay_track_id={} error: {:?}",
            self.client_connection_id, self.relay_track_id, e
          );
        }

        // Finish the subscription since the publisher is gone
        self.finish().await;
      }
    }
  }

  async fn handle_header(
    &self,
    header_info: HeaderInfo,
  ) -> Result<(StreamId, Arc<Mutex<TransportSendStream>>)> {
    // Handle the header information
    trace!("Handling header: {:?}", header_info);
    let stream_id = self.get_stream_id(&header_info);

    if let Ok(header_payload) = self.get_header_payload(&header_info).await {
      // hex dump the header payload
      trace!(
        "subscription::handle_object | header payload: {:?}",
        utils::bytes_to_hex(&header_payload)
      );

      let (pub_prio, group_id) = match &header_info {
        HeaderInfo::Subgroup { header } => (
          header
            .publisher_priority
            .unwrap_or(DEFAULT_PUBLISHER_PRIORITY),
          header.group_id,
        ),
        HeaderInfo::Fetch { .. } => (DEFAULT_PUBLISHER_PRIORITY, 0u64),
      };
      let (sub_prio, group_order) = {
        let state = self.subscription_state.read().await;
        (state.subscriber_priority, state.group_order)
      };
      let priority = compute_stream_priority(sub_prio, pub_prio, group_order, group_id);

      let send_stream = match self
        .subscriber
        .open_stream(&stream_id, header_payload, priority)
        .await
      {
        Ok(send_stream) => send_stream,
        Err(e) => {
          error!(
            "Failed to open stream {}: {:?} subscriber={} relay_track_id={}",
            stream_id, e, self.client_connection_id, self.relay_track_id
          );
          return Err(e);
        }
      };

      debug!("Created stream: {}", stream_id.get_stream_id());

      // Count every data stream opened for this subscription (PUBLISH_DONE
      // Stream Count), including subgroups that end up carrying no objects.
      self.opened_stream_count.fetch_add(1, Ordering::Relaxed);

      Ok((stream_id, send_stream.clone()))
    } else {
      error!(
        "Failed to serialize header payload for stream {} subscriber={} relay_track_id={}",
        stream_id, self.client_connection_id, self.relay_track_id
      );
      Err(anyhow::anyhow!(
        "Failed to serialize header payload for stream {} subscriber={} relay_track_id={}",
        stream_id,
        self.client_connection_id,
        self.relay_track_id
      ))
    }
  }

  async fn handle_object(
    &self,
    object: Object,
    previous_object_id: Option<u64>,
    stream_id: &StreamId,
    send_stream: Arc<Mutex<TransportSendStream>>,
  ) -> Result<()> {
    trace!(
      "Handling object relay_track_id={} location: {:?} stream_id={} diff_ms={}",
      self.relay_track_id,
      object.location,
      stream_id,
      utils::passed_time_since_start()
    );

    let object_location = object.location.clone();

    // This loop will keep the stream open and process incoming objects
    // TODO: revisit this logic to handle also fetch requests
    if let Ok(sub_object) = object.try_into_subgroup() {
      let has_properties = sub_object.properties.is_some();
      let object_bytes = match sub_object.serialize(previous_object_id, has_properties) {
        Ok(data) => data,
        Err(e) => {
          error!(
            "Error in serializing object before writing to stream for subscriber={} relay_track_id={}, location: {:?}, previous_object_id: {:?}, error: {:?}",
            self.client_connection_id, self.relay_track_id, object_location, previous_object_id, e
          );
          return Err(e.into());
        }
      };

      // uncomment to print hex dump of object bytes
      /*
      trace!(
        "subscription::handle_object | object bytes: {}",
        utils::bytes_to_hex(&object_bytes)
      );
      */

      self
        .subscriber
        .write_stream_object(
          stream_id,
          sub_object.object_id,
          object_bytes,
          Some(send_stream.clone()),
        )
        .await
        .map_err(|open_stream_err| {
          error!(
            "Error writing object to stream for subscriber={} relay_track_id={}, error: {:?}",
            self.client_connection_id, self.relay_track_id, open_stream_err
          );
          open_stream_err
        })
    } else {
      trace!(
        "Could not convert object to subgroup. stream_id: {:?} subscriber={} relay_track_id={}",
        stream_id, self.client_connection_id, self.relay_track_id
      );
      Err(anyhow::anyhow!(
        "Could not convert object to subgroup. stream_id: {:?} subscriber={} relay_track_id={}",
        stream_id,
        self.client_connection_id,
        self.relay_track_id
      ))
    }
  }

  async fn handle_stream_closed(&self, stream_id: &StreamId) -> Result<()> {
    // Handle the stream closed event
    trace!("Stream closed: {}", stream_id.get_stream_id());

    // remove the stream id from send_stream_last_object_ids immediately
    let mut send_stream_last_object_ids = self.send_stream_last_object_ids.write().await;
    send_stream_last_object_ids.remove(stream_id);
    drop(send_stream_last_object_ids); // Release the lock immediately

    // Perform graceful stream closure in a separate task to avoid blocking
    // the main subscription event loop. This is critical for real-time media streaming
    // where blocking operations can disrupt video flow timing (25fps = ~40ms intervals)
    let subscriber = self.subscriber.clone();
    let stream_id = stream_id.clone();
    let connection_id = self.client_connection_id;
    let relay_track_id = self.relay_track_id;

    tokio::spawn(async move {
      trace!(
        "Starting graceful stream closure in background: subscriber={} stream_id={} relay_track_id={}",
        connection_id, stream_id, relay_track_id
      );

      let res = subscriber.close_stream(&stream_id).await;
      if let Err(e) = res {
        warn!(
          "handle_stream_closed | error for subscriber={} stream_id={} relay_track_id={} error: {:?}",
          connection_id, stream_id, relay_track_id, e
        );
      } else if let Ok(closed) = res {
        if closed {
          trace!(
            "handle_stream_closed | successful for subscriber={} stream_id={} relay_track_id={}",
            connection_id, stream_id, relay_track_id
          );
        } else {
          trace!(
            "handle_stream_closed | stream not found for subscriber={} stream_id={} relay_track_id={}",
            connection_id, stream_id, relay_track_id
          );
        }
      }
    });

    // Return immediately to avoid blocking the event loop
    Ok(())
  }

  async fn get_header_payload(&self, header_info: &HeaderInfo) -> Result<Bytes> {
    let connection_id = self.client_connection_id;
    match header_info {
      HeaderInfo::Subgroup { header } => header.serialize(Some(self.relay_track_id)).map_err(|e| {
        error!(
          "Error serializing subgroup header: {:?} subscriber={} relay_track_id={}",
          e, connection_id, self.relay_track_id
        );
        e.into()
      }),
      HeaderInfo::Fetch {
        header,
        fetch_request: _,
      } => header.serialize().map_err(|e| {
        error!(
          "Error serializing fetch header: {:?} subscriber={} relay_track_id={}",
          e, connection_id, self.relay_track_id
        );
        e.into()
      }),
    }
  }

  fn get_stream_id(&self, header_info: &HeaderInfo) -> StreamId {
    utils::build_stream_id(self.relay_track_id, header_info)
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

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::control::constant::GroupOrder;

  #[test]
  fn test_highest_priority_near_i32_max() {
    let p = compute_stream_priority(0, 0, GroupOrder::Ascending, 0);
    assert!(
      p > 2_100_000_000,
      "highest priority should be near i32::MAX, got {p}"
    );
  }

  #[test]
  fn test_lowest_priority_near_i32_min() {
    let p = compute_stream_priority(255, 255, GroupOrder::Ascending, 0);
    assert!(
      p < -2_100_000_000,
      "lowest priority should be near i32::MIN, got {p}"
    );
  }

  #[test]
  fn test_ascending_lower_group_higher_priority() {
    let p0 = compute_stream_priority(0, 0, GroupOrder::Ascending, 0);
    let p1 = compute_stream_priority(0, 0, GroupOrder::Ascending, 1);
    assert!(p0 > p1, "group 0 should outrank group 1 in Ascending order");
  }

  #[test]
  fn test_descending_higher_group_higher_priority() {
    let p0 = compute_stream_priority(0, 0, GroupOrder::Descending, 0);
    let p1 = compute_stream_priority(0, 0, GroupOrder::Descending, 1);
    assert!(
      p1 > p0,
      "group 1 should outrank group 0 in Descending order"
    );
  }

  #[test]
  fn test_original_same_as_ascending() {
    for g in [0u64, 1, 100, 65535] {
      assert_eq!(
        compute_stream_priority(10, 20, GroupOrder::Original, g),
        compute_stream_priority(10, 20, GroupOrder::Ascending, g),
        "Original should behave like Ascending for group {g}"
      );
    }
  }

  #[test]
  fn test_subscriber_priority_dominates() {
    // sub=0,pub=255 must outrank sub=1,pub=0 regardless of group
    let high = compute_stream_priority(0, 255, GroupOrder::Ascending, 0);
    let low = compute_stream_priority(1, 0, GroupOrder::Ascending, 0);
    assert!(
      high > low,
      "subscriber priority must dominate publisher priority"
    );
  }

  #[test]
  fn test_publisher_priority_tie_break() {
    let high = compute_stream_priority(10, 0, GroupOrder::Ascending, 0);
    let low = compute_stream_priority(10, 1, GroupOrder::Ascending, 0);
    assert!(high > low, "lower pub_prio number = higher priority");
  }

  #[test]
  fn test_all_values_within_i32_range() {
    for sub in [0u8, 128, 255] {
      for pub_ in [0u8, 128, 255] {
        for &order in &[
          GroupOrder::Ascending,
          GroupOrder::Descending,
          GroupOrder::Original,
        ] {
          for group in [0u64, 1, 65534, 65535, 65536, u64::MAX] {
            let _ = compute_stream_priority(sub, pub_, order, group); // must not panic/overflow
          }
        }
      }
    }
  }
}
