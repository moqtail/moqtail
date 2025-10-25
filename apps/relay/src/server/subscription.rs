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
use crate::server::config::AppConfig;
use crate::server::object_logger::ObjectLogger;
use crate::server::stream_id::StreamId;
use crate::server::track::TrackEvent;
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
use moqtail::model::control::constant::SubscriptionForwardAction;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::control::subscribe_update::SubscribeUpdate;
use moqtail::model::data::object::Object;
use moqtail::model::parameter::constant::VersionSpecificParameterType;
use moqtail::model::parameter::version_parameter::VersionParameter;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::warn;
use tracing::{debug, error, info};
use wtransport::SendStream;

#[derive(Debug, Clone)]
pub struct SubscriptionState {
  pub subscriber_priority: u8,
  pub group_order: GroupOrder,
  pub forward: bool,
  pub filter_type: FilterType,
  pub start_location: Option<Location>,
  pub end_group: u64,
  pub subscribe_parameters: Vec<KeyValuePair>,
  pub last_forward_action: Option<SubscriptionForwardAction>,
  pub forward_action_group: u64,
}

impl SubscriptionState {
  pub fn new(
    subscriber_priority: u8,
    group_order: GroupOrder,
    forward: bool,
    filter_type: FilterType,
    start_location: Option<Location>,
    end_group: u64,
    subscribe_parameters: Vec<KeyValuePair>,
    last_forward_action: Option<SubscriptionForwardAction>,
    forward_action_group: u64,
  ) -> Self {
    Self {
      subscriber_priority,
      group_order,
      forward,
      filter_type,
      start_location,
      end_group,
      subscribe_parameters,
      last_forward_action,
      forward_action_group,
    }
  }

  pub fn get_forward_action_group(subscribe_parameters: &[KeyValuePair]) -> Option<u64> {
    // look up in the subscribe parameter
    for param in subscribe_parameters {
      // Try to convert the parameter to a VersionParameter
      if let Ok(version_param) = VersionParameter::try_from(param.clone()) {
        // Check if this is a ForwardActionGroup parameter
        if param.get_type() == VersionSpecificParameterType::ForwardActionGroup as u64
          && let VersionParameter::ForwardActionGroup { group_id } = version_param
        {
          return Some(group_id);
        }
      }
    }
    None
  }
}

impl From<Subscribe> for SubscriptionState {
  fn from(subscribe: Subscribe) -> Self {
    let forward = match subscribe.forward {
      SubscriptionForwardAction::DontForwardNow => false,
      SubscriptionForwardAction::ForwardNow => true,
      SubscriptionForwardAction::ForwardInFuture => false,
      SubscriptionForwardAction::DontForwardInFuture => true,
    };

    let forward_action_group =
      SubscriptionState::get_forward_action_group(&subscribe.subscribe_parameters).unwrap_or(0);

    Self::new(
      subscribe.subscriber_priority,
      subscribe.group_order,
      forward,
      subscribe.filter_type,
      subscribe.start_location,
      subscribe.end_group.unwrap_or(0),
      subscribe.subscribe_parameters,
      Some(subscribe.forward),
      forward_action_group,
    )
  }
}

#[derive(Debug, Clone)]
pub struct Subscription {
  pub request_id: u64,
  pub track_alias: u64,
  pub subscription_state: Arc<RwLock<SubscriptionState>>,
  subscriber: Arc<MOQTClient>,
  event_rx: Arc<Mutex<Option<UnboundedReceiver<TrackEvent>>>>,
  send_stream_last_object_ids: Arc<RwLock<HashMap<StreamId, Option<u64>>>>,
  finished: Arc<RwLock<bool>>, // Indicates if the subscription is finished
  #[allow(dead_code)]
  cache: TrackCache,
  client_connection_id: usize,
  object_logger: ObjectLogger,
  config: &'static AppConfig,
}

#[allow(clippy::too_many_arguments)]
impl Subscription {
  fn create_instance(
    track_alias: u64,
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
      track_alias,
      request_id,
      subscription_state: Arc::new(RwLock::new(subscribe_message.into())),
      subscriber,
      event_rx,
      send_stream_last_object_ids: Arc::new(RwLock::new(HashMap::new())),
      finished: Arc::new(RwLock::new(false)),
      cache,
      client_connection_id,
      object_logger: ObjectLogger::new(log_folder),
      config,
    }
  }
  pub fn new(
    track_alias: u64,
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
      track_alias,
      subscribe_message.request_id,
      subscribe_message,
      subscriber,
      event_rx,
      cache,
      client_connection_id,
      log_folder,
      config,
    );

    let mut instance = sub.clone();
    tokio::spawn(async move {
      loop {
        {
          let is_finished = instance.finished.read().await;
          if *is_finished {
            break;
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

  // This method updates the subscribe message with the new subscribe update
  // It ensures that the Start Location does not decrease and the End Group does not increase
  // Returns Ok if the update is successful
  // Returns error if the update is invalid
  pub async fn update_subscription(&self, subscribe_update: SubscribeUpdate) -> Result<()> {
    let mut state = self.subscription_state.write().await;
    // map subscribe_update fields to subscribe_message
    // The Start Location	MUST NOT decrease and the End Group MUST NOT increase.

    info!("Updating subscription {:?}", subscribe_update);

    let mut discard_end_group = false;
    if matches!(
      subscribe_update.forward,
      SubscriptionForwardAction::DontForwardInFuture
    ) || matches!(
      subscribe_update.forward,
      SubscriptionForwardAction::ForwardInFuture
    ) {
      // if forward action is in the future we expect to get a sub parameter that contains the group id
      let forward_action_group =
        SubscriptionState::get_forward_action_group(&subscribe_update.subscribe_parameters);
      if forward_action_group.is_some() {
        state.forward_action_group = forward_action_group.unwrap_or(0);
      } else {
        // if the group id is not sent as a parameter we accept end_group
        state.forward_action_group = subscribe_update.end_group;
        // reset end group
        discard_end_group = true;
      }
    } else {
      state.forward = matches!(
        subscribe_update.forward,
        SubscriptionForwardAction::ForwardNow
      );
    }
    let discard_end_group = discard_end_group;

    if subscribe_update.start_location > state.start_location.clone().unwrap_or_default()
      || (!discard_end_group
        && subscribe_update.end_group > 0
        && subscribe_update.end_group - 1 < state.end_group)
    {
      // invalid update
      return Err(anyhow::anyhow!(
        "Invalid SubscribeUpdate: Start Location cannot decrease and End Group cannot increase"
      ));
    }

    // update subscription state
    state.start_location = Some(subscribe_update.start_location);
    state.subscriber_priority = subscribe_update.subscriber_priority;
    state.last_forward_action = Some(subscribe_update.forward);
    if !discard_end_group && subscribe_update.end_group > 0 {
      state.end_group = subscribe_update.end_group - 1; // end group + 1 is sent in sub. update
    }
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

    info!("updated state for {} state: {:?}", self.track_alias, state);
    Ok(())
  }

  pub async fn finish(&self) {
    if *self.finished.read().await {
      // already finished
      return;
    }

    info!(
      "Finishing subscription for subscriber: {} and track: {}",
      self.client_connection_id, self.track_alias
    );

    let mut finished = self.finished.write().await;
    *finished = true;

    let mut receiver_guard = self.event_rx.lock().await;
    let _ = receiver_guard.take(); // This replaces the Some(receiver) with None
    drop(receiver_guard); // Release the lock

    info!(
      "Subscription finished for subscriber: {} and track: {}",
      self.client_connection_id, self.track_alias
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
      let track_alias = self.track_alias;

      // Spawn background task for graceful stream cleanup
      tokio::spawn(async move {
        info!(
          "Starting background cleanup of {} streams for subscriber: {} track: {}",
          stream_ids.len(),
          connection_id,
          track_alias
        );

        for stream_id in stream_ids.iter() {
          if let Err(e) = subscriber.close_stream(stream_id).await {
            warn!(
              "Background stream cleanup error for subscriber: {} stream_id: {} track: {} error: {:?}",
              connection_id, stream_id, track_alias, e
            );
          } else {
            debug!(
              "Background stream cleanup successful for subscriber: {} stream_id: {} track: {}",
              connection_id, stream_id, track_alias
            );
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

  async fn receive(&mut self) {
    debug!(
      "Receiving for subscriber: {} track: {}",
      self.client_connection_id, self.track_alias
    );
    let mut event_rx_guard = self.event_rx.lock().await;

    if let Some(ref mut event_rx) = *event_rx_guard {
      match event_rx.recv().await {
        Some(event) => {
          debug!(
            "Event received for subscriber: {} track: {} event: {:?}",
            self.client_connection_id, self.track_alias, event
          );
          if *self.finished.read().await {
            return;
          }

          match event {
            TrackEvent::Object {
              object,
              stream_id,
              header_info,
            } => {
              let object_received_time = utils::passed_time_since_start();

              let mut forward = true;
              {
                let state = self.subscription_state.read().await;
                if let Some(start) = &state.start_location
                  && object.location < *start
                {
                  return;
                }

                // if the object is after the end group, finish the subscription
                if state.end_group > 0 && object.location.group > state.end_group {
                  info!(
                    "Finishing subscription for subscriber: {} track: {}",
                    self.client_connection_id, self.track_alias
                  );
                  self.finish().await;
                  return;
                }

                forward = state.forward;

                if let Some(last_forward_action) = &state.last_forward_action.clone()
                  && (matches!(
                    &last_forward_action,
                    SubscriptionForwardAction::DontForwardInFuture
                      | SubscriptionForwardAction::ForwardInFuture
                  ))
                  && state.forward_action_group <= object.location.group
                {
                  info!(
                    "Forward action triggered {:?} track {} object location {:?} forward_action_group {}",
                    &last_forward_action,
                    self.track_alias,
                    &object.location,
                    state.forward_action_group
                  );
                  drop(state);
                  let mut state = self.subscription_state.write().await;
                  state.forward = matches!(
                    &last_forward_action,
                    SubscriptionForwardAction::ForwardInFuture
                  );
                  forward = state.forward;
                  state.last_forward_action = None;
                  state.forward_action_group = 0;
                }
              }

              if !forward {
                return;
              }

              // Handle header info if this is the first object
              let send_stream = if let Some(header) = header_info {
                if let HeaderInfo::Subgroup {
                  header: _subgroup_header,
                } = header
                {
                  info!(
                    "Creating stream - subscriber: {} track: {} now: {} received time: {} object: {:?}",
                    self.client_connection_id,
                    self.track_alias,
                    utils::passed_time_since_start(),
                    object_received_time,
                    object.location
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
                      self.track_alias,
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
                  "Received Object event: subscriber: {} stream_id: {} track: {} previous_object_id: {:?}",
                  self.client_connection_id, stream_id, self.track_alias, previous_object_id
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
                  let mut send_stream_last_object_ids =
                    self.send_stream_last_object_ids.write().await;
                  send_stream_last_object_ids
                    .insert(stream_id.clone(), Some(object.location.object));
                }

                if self.config.enable_object_logging {
                  self
                    .object_logger
                    .log_subscription_object(
                      self.track_alias,
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
                  self.track_alias,
                  object.location,
                  utils::passed_time_since_start(),
                  object_received_time
                );
              }
            }
            TrackEvent::StreamClosed { stream_id } => {
              info!(
                "Received StreamClosed event: subscriber: {} stream_id: {} track: {}",
                self.client_connection_id, stream_id, self.track_alias
              );
              let _ = self.handle_stream_closed(&stream_id).await;
            }
            TrackEvent::PublisherDisconnected { reason } => {
              info!(
                "Received PublisherDisconnected event: subscriber: {}, reason: {} track: {}",
                self.client_connection_id, reason, self.track_alias
              );

              // Send PublishDone message and finish the subscription
              if let Err(e) = self
                .send_publish_done(PublishDoneStatusCode::TrackEnded, &reason)
                .await
              {
                error!(
                  "Failed to send PublishDone for publisher disconnect: subscriber: {} track: {} error: {:?}",
                  self.client_connection_id, self.track_alias, e
                );
              }

              // Finish the subscription since the publisher is gone
              self.finish().await;
            }
          }
        }
        None => {
          // For unbounded receivers, recv() returns None when the channel is closed
          // The channel is closed, we should finish the subscription
          info!(
            "Event receiver closed for subscriber: {} track: {}, finishing subscription",
            self.client_connection_id, self.track_alias
          );
          self.finish().await;
        }
      }
    } else {
      // No receiver available, subscription has been finished
      self.finish().await;
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
            stream_id, e, self.client_connection_id, self.track_alias
          );
          return Err(e);
        }
      };

      info!("Created stream: {}", stream_id.get_stream_id());

      Ok((stream_id, send_stream.clone()))
    } else {
      error!(
        "Failed to serialize header payload for stream {} subscriber: {} track: {}",
        stream_id, self.client_connection_id, self.track_alias
      );
      Err(anyhow::anyhow!(
        "Failed to serialize header payload for stream {} subscriber: {} track: {}",
        stream_id,
        self.client_connection_id,
        self.track_alias
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

    // This loop will keep the stream open and process incoming objects
    // TODO: revisit this logic to handle also fetch requests
    if let Ok(sub_object) = object.try_into_subgroup() {
      let has_extensions = sub_object.extension_headers.is_some();
      let object_bytes = match sub_object.serialize(previous_object_id, has_extensions) {
        Ok(data) => data,
        Err(e) => {
          error!(
            "Error in serializing object before writing to stream for subscriber {} track: {}, error: {:?}",
            self.client_connection_id, self.track_alias, e
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
        .write_object_to_stream(
          stream_id,
          sub_object.object_id,
          object_bytes,
          Some(send_stream.clone()),
        )
        .await
        .map_err(|open_stream_err| {
          error!(
            "Error writing object to stream for subscriber {} track: {}, error: {:?}",
            self.client_connection_id, self.track_alias, open_stream_err
          );
          open_stream_err
        })
    } else {
      debug!(
        "Could not convert object to subgroup. stream_id: {:?} subscriber: {} track: {}",
        stream_id, self.client_connection_id, self.track_alias
      );
      Err(anyhow::anyhow!(
        "Could not convert object to subgroup. stream_id: {:?} subscriber: {} track: {}",
        stream_id,
        self.client_connection_id,
        self.track_alias
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
    let stream_id_clone = stream_id.clone();
    let connection_id = self.client_connection_id;
    let track_alias = self.track_alias;

    tokio::spawn(async move {
      debug!(
        "Starting graceful stream closure in background: subscriber: {} stream_id: {} track: {}",
        connection_id, stream_id_clone, track_alias
      );

      if let Err(e) = subscriber.close_stream(&stream_id_clone).await {
        // Log the error but don't propagate it since this is background cleanup
        debug!(
          "Background stream closure completed with error: subscriber: {} stream_id: {} track: {} error: {:?}",
          connection_id, stream_id_clone, track_alias, e
        );
      } else {
        debug!(
          "Background stream closure completed successfully: subscriber: {} stream_id: {} track: {}",
          connection_id, stream_id_clone, track_alias
        );
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
          e, connection_id, self.track_alias
        );
        e.into()
      }),
      HeaderInfo::Fetch {
        header,
        fetch_request: _,
      } => header.serialize().map_err(|e| {
        error!(
          "Error serializing fetch header: {:?} subscriber: {} track: {}",
          e, connection_id, self.track_alias
        );
        e.into()
      }),
    }
  }

  fn get_stream_id(&self, header_info: &HeaderInfo) -> StreamId {
    utils::build_stream_id(self.track_alias, header_info)
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
      self.client_connection_id, self.track_alias, self.request_id
    );

    Ok(())
  }
}
