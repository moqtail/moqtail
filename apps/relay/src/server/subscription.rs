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
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::object::Object;
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
pub struct Subscription {
  pub track_alias: u64,
  pub subscribe_message: Subscribe,
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
      subscribe_message,
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

  pub async fn finish(&mut self) {
    let mut is_finished = self.finished.write().await;
    *is_finished = true;
    drop(is_finished); // Explicitly drop the lock to allow other tasks to proceed

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
    let mut event_rx_guard = self.event_rx.lock().await;

    if let Some(ref mut event_rx) = *event_rx_guard {
      match event_rx.recv().await {
        Some(event) => {
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
              let mut is_finished = self.finished.write().await;
              *is_finished = true;
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
          let mut is_finished = self.finished.write().await;
          *is_finished = true;
        }
      }
    } else {
      // No receiver available, subscription has been finished
      let mut is_finished = self.finished.write().await;
      *is_finished = true;
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

      // print hex dump of object bytes
      debug!(
        "subscription::handle_object | object bytes: {}",
        utils::bytes_to_hex(&object_bytes)
      );

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
      self.subscribe_message.request_id,
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
      self.client_connection_id, self.track_alias, self.subscribe_message.request_id
    );

    Ok(())
  }
}
