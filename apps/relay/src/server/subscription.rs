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
use crate::server::client::switch_context::SwitchStatus;
use crate::server::config::AppConfig;
use crate::server::object_logger::ObjectLogger;
use crate::server::stream_id::StreamId;
use crate::server::track::TrackEvent;
use crate::server::track_cache::CacheConsumeEvent;
use crate::server::track_cache::TrackCache;
use crate::server::utils;
use anyhow::Result;
use bytes::Bytes;
use moqtail::model::common::location::Location;
use moqtail::model::common::pair::KeyValuePair;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::FilterType;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::control::subscribe_update::SubscribeUpdate;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::data::object::Object;
use moqtail::model::data::subgroup_header::SubgroupHeader;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::warn;
use tracing::{debug, error, info};
use wtransport::SendStream;

#[derive(Debug, Clone)]
pub struct SubscriptionState {
  pub subscriber_priority: u8,
  pub _group_order: GroupOrder,
  pub forward: bool,
  pub _filter_type: FilterType,
  pub start_location: Option<Location>,
  pub end_group: u64,
  pub subscribe_parameters: Vec<KeyValuePair>,
  pub last_sent_max_location: Option<Location>,
  pub last_received_object_location: Option<Location>,
  pub is_joining: bool,
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

impl From<Subscribe> for SubscriptionState {
  fn from(subscribe: Subscribe) -> Self {
    Self {
      subscriber_priority: subscribe.subscriber_priority,
      _group_order: subscribe.group_order,
      forward: subscribe.forward,
      _filter_type: subscribe.filter_type,
      start_location: subscribe.start_location,
      end_group: subscribe.end_group.unwrap_or(0),
      subscribe_parameters: subscribe.subscribe_parameters,
      last_sent_max_location: None,
      last_received_object_location: None,
      is_joining: false,
    }
  }
}

#[derive(Debug, Clone)]
pub struct Subscription {
  pub request_id: u64,
  track_alias_ref: Arc<AtomicU64>,
  pub full_track_name: FullTrackName,
  pub subscription_state: Arc<RwLock<SubscriptionState>>,
  subscriber: Arc<MOQTClient>,
  event_rx: Arc<Mutex<Option<UnboundedReceiver<TrackEvent>>>>,
  send_stream_last_object_ids: Arc<RwLock<HashMap<StreamId, Option<u64>>>>,
  finished: Arc<AtomicBool>,
  #[allow(dead_code)]
  cache: TrackCache,
  client_connection_id: usize,
  object_logger: ObjectLogger,
  config: &'static AppConfig,
  check_switch_context_on_next_object: Arc<AtomicBool>,
}

#[allow(clippy::too_many_arguments)]
impl Subscription {
  /// Get the current track alias value (shared with SubscriptionManager).
  fn track_alias(&self) -> u64 {
    self.track_alias_ref.load(Ordering::Relaxed)
  }

  fn create_instance(
    track_alias_ref: Arc<AtomicU64>,
    full_track_name: FullTrackName,
    request_id: u64,
    subscribe_message: Subscribe,
    subscriber: Arc<MOQTClient>,
    event_rx: Arc<Mutex<Option<UnboundedReceiver<TrackEvent>>>>,
    cache: TrackCache,
    client_connection_id: usize,
    log_folder: String,
    config: &'static AppConfig,
  ) -> Self {
    Self {
      track_alias_ref,
      full_track_name,
      request_id,
      subscription_state: Arc::new(RwLock::new(subscribe_message.into())),
      subscriber,
      event_rx,
      send_stream_last_object_ids: Arc::new(RwLock::new(HashMap::new())),
      finished: Arc::new(AtomicBool::new(false)),
      cache,
      client_connection_id,
      object_logger: ObjectLogger::new(log_folder),
      config,
      check_switch_context_on_next_object: Arc::new(AtomicBool::new(false)),
    }
  }
  pub fn new(
    track_alias: Arc<AtomicU64>,
    full_track_name: FullTrackName,
    subscribe_message: Subscribe,
    subscriber: Arc<MOQTClient>,
    event_rx: UnboundedReceiver<TrackEvent>,
    cache: TrackCache,
    client_connection_id: usize,
    log_folder: String,
    config: &'static AppConfig,
  ) -> Self {
    let event_rx = Arc::new(Mutex::new(Some(event_rx)));
    let sub = Self::create_instance(
      track_alias.clone(),
      full_track_name,
      subscribe_message.request_id,
      subscribe_message,
      subscriber,
      event_rx,
      cache.clone(),
      client_connection_id,
      log_folder,
      config,
    );

    let track_alias = track_alias.load(Ordering::Relaxed);

    info!(
      "Created new Subscription instance for subscriber: {} track: {} subscription state: {:?}",
      client_connection_id, track_alias, sub.subscription_state
    );

    let mut instance = sub.clone();

    tokio::spawn(async move {
      loop {
        if instance.is_finished().await {
          break;
        }

        // Handle joining state
        {
          let state = instance.subscription_state.read().await;
          let start_location = state.start_location.clone();
          let last_received_object_location_opt = state.last_received_object_location.clone();
          let is_joining = state.is_joining;
          drop(state);
          if is_joining && start_location.is_some() {
            let start_location = start_location.unwrap_or_default();
            if let Some(last_received_object_location) = last_received_object_location_opt {
              info!(
                "Joining state - subscriber: {} track: {} from location: {:?} to last received location: {:?}",
                instance.client_connection_id,
                track_alias,
                start_location,
                last_received_object_location
              );
              if last_received_object_location > start_location {
                let mut object_receiver = cache
                  .read_objects(start_location, last_received_object_location, false)
                  .await;

                let mut last_group: u64 = u64::MAX;
                let mut last_stream_id: Option<StreamId> = None;

                loop {
                  match object_receiver.recv().await {
                    Some(event) => match event {
                      CacheConsumeEvent::NoObject => {
                        // there is no object found
                        break;
                      }
                      CacheConsumeEvent::Object(object) => {
                        let (header_info, stream_id) = if last_group == u64::MAX
                          || object.group_id > last_group
                        {
                          // create a subgroup header and send a track event

                          // TODO: check this. If is_some returns true, we may not need
                          // to check the length.
                          let has_extensions = object.extension_headers.as_ref().is_some();

                          // create a fake subgroup header using the object attributes
                          // TODO: It think contains_end_of_group should be checked by looking at
                          // the last object. Need to look into the draft.
                          let subgroup_header = HeaderInfo::Subgroup {
                            header: SubgroupHeader::new_with_explicit_id(
                              track_alias,
                              object.group_id,
                              object.subgroup_id,
                              object.publisher_priority,
                              has_extensions,
                              false,
                            ),
                          };
                          info!(
                            "FROM CACHE: Joining state - subscriber: {} track: {} sending subgroup header: {:?}",
                            instance.client_connection_id, track_alias, subgroup_header
                          );
                          last_group = object.group_id;
                          let stream_id = instance.get_stream_id(&subgroup_header);
                          last_stream_id = Some(stream_id);

                          (Some(subgroup_header), last_stream_id.clone())
                        } else {
                          (None, last_stream_id.clone())
                        };

                        let the_object = Object::try_from_fetch(object, track_alias).unwrap();

                        let track_event = TrackEvent::SubgroupObject {
                          stream_id: stream_id.unwrap(),
                          object: the_object,
                          header_info,
                        };
                        info!(
                          "Joining state - subscriber: {} track: {} sending object location: {:?}",
                          instance.client_connection_id, track_alias, track_event
                        );
                        instance.handle_track_event(track_event).await;
                      }
                      CacheConsumeEvent::EndLocation(_) => {}
                    },
                    None => {
                      warn!("handle_fetch_messages | No object.");
                      break;
                    }
                  }
                }
              }
            }
            let mut state = instance.subscription_state.write().await;
            state.is_joining = false;
            info!(
              "Finished joining state for subscriber: {} track: {}",
              instance.client_connection_id, track_alias
            );
          }
        }

        tokio::select! {
          biased;
          _ = instance.receive() => {
            continue;
          }
          // 1 second timeout to check if the subscription is still valid
          _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
            // TODO: implement max timeout here
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

  // Returns true if the subscription is active (not finished and forwarding objects)
  pub async fn is_active(&self) -> bool {
    !self.is_finished().await && self.is_forwarding().await
  }

  // This method updates the subscribe message with the new subscribe update
  // It ensures that the Start Location does not decrease and the End Group does not increase
  // Returns Ok if the update is successful
  // Returns error if the update is invalid
  pub async fn update_subscription(&self, subscribe_update: SubscribeUpdate) -> Result<()> {
    let mut state = self.subscription_state.write().await;
    // map subscribe_update fields to subscribe_message

    // The Start Location MUST NOT decrease
    // and the End Group MUST NOT increase.
    // In Draft-15 end group can be increased or decreased.
    if subscribe_update.start_location < state.start_location.clone().unwrap_or_default() {
      // invalid update
      return Err(anyhow::anyhow!(
        "Invalid SubscribeUpdate: Start Location cannot decrease. Current start location: {:?} Subscribe Update Start Location: {:?}",
        state.start_location,
        subscribe_update.start_location
      ));
    }

    // update subscription state
    state.start_location = Some(subscribe_update.start_location);
    state.subscriber_priority = subscribe_update.subscriber_priority;
    state.forward = subscribe_update.forward;
    state.end_group = subscribe_update.end_group;

    // update parameters. If a parameter included in SUBSCRIBE is not present in
    // SUBSCRIBE_UPDATE, its value remains unchanged.  There is no mechanism
    // to remove a parameter from a subscription.
    for param in subscribe_update.subscribe_parameters {
      if let Some(existing_param) = state
        .subscribe_parameters
        .iter_mut()
        .find(|p| p.is_same_type(&param))
      {
        *existing_param = param;
      } else {
        state.subscribe_parameters.push(param);
      }
    }

    info!(
      "update_subscription | new subscription state for {}: {:?}",
      self.track_alias(),
      state
    );
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
      "Finishing subscription for subscriber: {} and track: {}",
      self.client_connection_id,
      self.track_alias()
    );

    let mut receiver_guard = self.event_rx.lock().await;
    let _ = receiver_guard.take(); // This replaces the Some(receiver) with None
    drop(receiver_guard); // Release the lock

    info!(
      "Subscription finished for subscriber: {} and track: {}",
      self.client_connection_id,
      self.track_alias()
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
      let track_alias = self.track_alias();

      // Spawn background task for graceful stream cleanup
      tokio::spawn(async move {
        info!(
          "Starting background cleanup of {} streams for subscriber: {} track: {}",
          stream_ids.len(),
          connection_id,
          track_alias
        );

        for stream_id in stream_ids.iter() {
          let res = subscriber.close_stream(stream_id).await;
          if let Err(e) = res {
            warn!(
              "Background stream cleanup error for subscriber: {} stream_id: {} track: {} error: {:?}",
              connection_id, stream_id, track_alias, e
            );
          } else if let Ok(closed) = res {
            if closed {
              debug!(
                "Background stream cleanup successful for subscriber: {} stream_id: {} track: {}",
                connection_id, stream_id, track_alias
              );
            } else {
              debug!(
                "Background stream cleanup: stream not found for subscriber: {} stream_id: {} track: {}",
                connection_id, stream_id, track_alias
              );
            }
          }
        }

        info!(
          "Background cleanup completed for subscriber: {} track: {} ({} streams)",
          connection_id,
          track_alias,
          stream_ids.len()
        );
      });
    }
  }

  // Notify the subscription to check the switch context on the next object
  pub async fn notify_switch(&self) {
    info!(
      "Notifying subscription to check switch context on next object for subscriber: {} track: {}",
      self.client_connection_id,
      self.track_alias()
    );
    self
      .check_switch_context_on_next_object
      .store(true, std::sync::atomic::Ordering::Relaxed);
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

        if let Some(current_track_name) = self.subscriber.switch_context.get_current().await {
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
            "check_switch_context: Setting track to Current for subscriber: {} track: {} object location group: {}",
            self.client_connection_id,
            self.track_alias(),
            object_location.group
          );
          subscriber
            .switch_context
            .add_or_update_switch_item(full_track_name.clone(), SwitchStatus::Current)
            .await;

          // set forward to true and set start group the next group
          let mut state = self.subscription_state.write().await;
          state.forward = true;

          state.is_joining = true;

          if new_start_location.is_some() {
            state.start_location = new_start_location;
          } else {
            state.start_location = Some(Location {
              object: 0,
              group: object_location.group + 1,
            });
          }

          state.end_group = 0; // remove end group limit

          info!(
            "check_switch_context: Will forward objects for subscriber: {} track: {} starting from group: {}",
            self.client_connection_id,
            self.track_alias(),
            state.start_location.as_ref().unwrap().group
          );
        } else {
          // Do not forward objects for Next status until switch condition is met
          // set forward to false if it is true
          if self.is_forwarding().await {
            info!(
              "check_switch_context: Setting forward to false for Next track for subscriber: {} track: {} obkject location group: {}",
              self.client_connection_id,
              self.track_alias(),
              object_location.group
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
        // set forward to false if it is true
        if self.is_forwarding().await {
          info!(
            "check_switch_context: Setting end group to {} for None track for subscriber: {} track: {}",
            object_location.group,
            self.client_connection_id,
            self.track_alias()
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
    debug!(
      "Receiving for subscriber: {} track: {}",
      self.client_connection_id,
      self.track_alias()
    );
    let mut event_rx_guard = self.event_rx.lock().await;

    if let Some(ref mut event_rx) = *event_rx_guard {
      match event_rx.recv().await {
        Some(event) => {
          if self.finished.load(Ordering::Relaxed) {
            return;
          }
          self.handle_track_event(event).await;
        }
        None => {
          // For unbounded receivers, recv() returns None when the channel is closed
          // The channel is closed, we should finish the subscription
          info!(
            "Event receiver closed for subscriber: {} track: {}, finishing subscription",
            self.client_connection_id,
            self.track_alias()
          );
          self.finish().await;
        }
      }
    } else {
      // No receiver available, subscription has been finished
      self.finish().await;
    }
  }

  async fn handle_track_event(&self, event: TrackEvent) {
    debug!(
      "Event received for subscriber: {} track: {} event: {:?}",
      self.client_connection_id,
      self.track_alias(),
      event
    );
    match event {
      TrackEvent::SubgroupObject {
        object,
        stream_id,
        header_info,
      } => {
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
          // Check wheter this track is in a switch context and update forward state
          if !self.check_switch_context(&object.location).await {
            // if this returns false, do not start the stream
            info!(
              "Not forwarding object for subscriber: {} track: {} due to switch context state",
              self.client_connection_id,
              self.track_alias()
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
            debug!(
              "Object before start location for subscriber: {} track: {} object location: {:?} start location: {:?}",
              self.client_connection_id,
              self.track_alias(),
              object.location,
              start
            );
            return;
          }

          if state.end_group > 0 && object.location.group > state.end_group {
            /* With Draft-15, the end group can be increased or decreased.
            TODO: Remove the following code after draft-15 support.
            info!(
              "Finishing subscription for subscriber: {} track: {}",
              self.client_connection_id, self.track_alias
            );
            self.finish().await;
            */
            debug!(
              "Object beyond end group for subscriber: {} track: {} object location: {:?} end group: {}",
              self.client_connection_id,
              self.track_alias(),
              object.location,
              state.end_group
            );
            return;
          }

          if !state.forward {
            return;
          }
        }

        // Handle header info if this is the first object
        let mut send_stream = if let Some(header) = header_info {
          if let HeaderInfo::Subgroup { header: _ } = header {
            info!(
              "Creating stream - subscriber: {} track: {} now: {} received time: {} object: {:?} header: {:?}",
              self.client_connection_id,
              self.track_alias(),
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
              info!(
                "Stream created - subscriber: {} stream_id: {} track: {} now: {} received time: {} object: {:?}",
                self.client_connection_id,
                stream_id,
                self.track_alias(),
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
          self.subscriber.get_stream(&stream_id).await
        };

        if send_stream.is_none() {
          // wait a little bit and try again
          warn!(
            "Send stream not found, retrying - subscriber: {} stream_id: {} track: {} now: {} received time: {} object: {:?}",
            self.client_connection_id,
            stream_id,
            self.track_alias(),
            utils::passed_time_since_start(),
            object_received_time,
            object.location
          );
          tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
          send_stream = self.subscriber.get_stream(&stream_id).await;
        }

        if let Some(send_stream) = send_stream {
          // Get the previous object ID for this stream
          let previous_object_id = {
            let send_stream_last_object_ids = self.send_stream_last_object_ids.read().await;
            send_stream_last_object_ids
              .get(&stream_id)
              .cloned()
              .flatten()
          };

          debug!(
            "Received Object event: subscriber: {} stream_id: {} track: {} previous_object_id: {:?} object: {:?} now: {} received time: {}",
            self.client_connection_id,
            stream_id,
            self.track_alias(),
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
                self.track_alias(),
                self.client_connection_id,
                &object,
                send_status,
                object_received_time,
              )
              .await;
          }
        } else {
          error!(
            "Received Object event without a valid send stream for subscriber: {} stream_id: {} track: {} object: {:?} now: {} received time: {}",
            self.client_connection_id,
            stream_id,
            self.track_alias(),
            object.location,
            utils::passed_time_since_start(),
            object_received_time
          );
        }
      }
      TrackEvent::DatagramObject { object } => {
        // Handle datagram object - serialize full MOQT datagram format
        // Must include type, track_alias, group_id, object_id, publisher_priority, and payload
        match object.serialize() {
          Ok(serialized_bytes) => {
            if let Err(e) = self
              .subscriber
              .write_datagram_object(serialized_bytes)
              .await
            {
              error!("Failed to write datagram object: {:?}", e);
            }
          }
          Err(e) => {
            error!("Failed to serialize datagram object: {:?}", e);
          }
        }
      }
      TrackEvent::StreamClosed { stream_id } => {
        info!(
          "Received StreamClosed event: subscriber: {} stream_id: {} track: {}",
          self.client_connection_id,
          stream_id,
          self.track_alias()
        );
        let _ = self.handle_stream_closed(&stream_id).await;
      }
      TrackEvent::PublisherDisconnected { reason } => {
        info!(
          "Received PublisherDisconnected event: subscriber: {}, reason: {} track: {}",
          self.client_connection_id,
          reason,
          self.track_alias()
        );

        // Send PublishDone message and finish the subscription
        if let Err(e) = self
          .send_publish_done(PublishDoneStatusCode::TrackEnded, &reason)
          .await
        {
          error!(
            "Failed to send PublishDone for publisher disconnect: subscriber: {} track: {} error: {:?}",
            self.client_connection_id,
            self.track_alias(),
            e
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
  ) -> Result<(StreamId, Arc<Mutex<SendStream>>)> {
    // Handle the header information
    debug!("Handling header: {:?}", header_info);
    let stream_id = self.get_stream_id(&header_info);

    if let Ok(header_payload) = self.get_header_payload(&header_info).await {
      // hex dump the header payload
      debug!(
        "subscription::handle_object | header payload: {:?}",
        utils::bytes_to_hex(&header_payload)
      );

      // set priority based on the current time
      // TODO: revisit this logic to set priority based on the subscription
      let priority = i32::MAX - (utils::passed_time_since_start() % i32::MAX as u128) as i32;

      let send_stream = match self
        .subscriber
        .open_stream(&stream_id, header_payload, priority)
        .await
      {
        Ok(send_stream) => send_stream,
        Err(e) => {
          error!(
            "Failed to open stream {}: {:?} subscriber: {} track: {}",
            stream_id,
            e,
            self.client_connection_id,
            self.track_alias()
          );
          return Err(e);
        }
      };

      info!("Created stream: {}", stream_id.get_stream_id());

      Ok((stream_id, send_stream.clone()))
    } else {
      error!(
        "Failed to serialize header payload for stream {} subscriber: {} track: {}",
        stream_id,
        self.client_connection_id,
        self.track_alias()
      );
      Err(anyhow::anyhow!(
        "Failed to serialize header payload for stream {} subscriber: {} track: {}",
        stream_id,
        self.client_connection_id,
        self.track_alias()
      ))
    }
  }

  async fn handle_object(
    &self,
    object: Object,
    previous_object_id: Option<u64>,
    stream_id: &StreamId,
    send_stream: Arc<Mutex<SendStream>>,
  ) -> Result<()> {
    debug!(
      "Handling object track: {} location: {:?} stream_id: {} diff_ms: {}",
      object.track_alias,
      object.location,
      stream_id,
      utils::passed_time_since_start()
    );

    let object_location = object.location.clone();

    // This loop will keep the stream open and process incoming objects
    // TODO: revisit this logic to handle also fetch requests
    if let Ok(sub_object) = object.try_into_subgroup() {
      let has_extensions = sub_object.extension_headers.is_some();
      let object_bytes = match sub_object.serialize(previous_object_id, has_extensions) {
        Ok(data) => data,
        Err(e) => {
          error!(
            "Error in serializing object before writing to stream for subscriber {} track: {}, location: {:?}, previous_object_id: {:?}, error: {:?}",
            self.client_connection_id,
            self.track_alias(),
            object_location,
            previous_object_id,
            e
          );
          return Err(e.into());
        }
      };

      // uncomment to print hex dump of object bytes
      /*
      debug!(
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
            "Error writing object to stream for subscriber {} track: {}, error: {:?}",
            self.client_connection_id,
            self.track_alias(),
            open_stream_err
          );
          open_stream_err
        })
    } else {
      debug!(
        "Could not convert object to subgroup. stream_id: {:?} subscriber: {} track: {}",
        stream_id,
        self.client_connection_id,
        self.track_alias()
      );
      Err(anyhow::anyhow!(
        "Could not convert object to subgroup. stream_id: {:?} subscriber: {} track: {}",
        stream_id,
        self.client_connection_id,
        self.track_alias()
      ))
    }
  }

  async fn handle_stream_closed(&self, stream_id: &StreamId) -> Result<()> {
    // Handle the stream closed event
    debug!("Stream closed: {}", stream_id.get_stream_id());

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
    let track_alias = self.track_alias();

    tokio::spawn(async move {
      debug!(
        "Starting graceful stream closure in background: subscriber: {} stream_id: {} track: {}",
        connection_id, stream_id, track_alias
      );

      let res = subscriber.close_stream(&stream_id).await;
      if let Err(e) = res {
        warn!(
          "handle_stream_closed | error for subscriber: {} stream_id: {} track: {} error: {:?}",
          connection_id, stream_id, track_alias, e
        );
      } else if let Ok(closed) = res {
        if closed {
          debug!(
            "handle_stream_closed | successful for subscriber: {} stream_id: {} track: {}",
            connection_id, stream_id, track_alias
          );
        } else {
          debug!(
            "handle_stream_closed | stream not found for subscriber: {} stream_id: {} track: {}",
            connection_id, stream_id, track_alias
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
      HeaderInfo::Subgroup { header } => header.serialize().map_err(|e| {
        error!(
          "Error serializing subgroup header: {:?} subscriber: {} track: {}",
          e,
          connection_id,
          self.track_alias()
        );
        e.into()
      }),
      HeaderInfo::Fetch {
        header,
        fetch_request: _,
      } => header.serialize().map_err(|e| {
        error!(
          "Error serializing fetch header: {:?} subscriber: {} track: {}",
          e,
          connection_id,
          self.track_alias()
        );
        e.into()
      }),
    }
  }

  fn get_stream_id(&self, header_info: &HeaderInfo) -> StreamId {
    utils::build_stream_id(self.track_alias(), header_info)
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
      self.request_id,
      status_code,
      0, // stream_count - set to 0 as track is ending
      reason_phrase,
    );

    self
      .subscriber
      .queue_message(ControlMessage::PublishDone(Box::new(publish_done)))
      .await;

    info!(
      "Sent PublishDone to subscriber {} track: {} for request_id {}",
      self.client_connection_id,
      self.track_alias(),
      self.request_id
    );

    Ok(())
  }
}
