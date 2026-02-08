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
use crate::server::session_context::SessionContext;
use crate::server::stream_id::StreamId;
use crate::server::track_cache::CacheConsumeEvent;
use crate::server::utils::build_stream_id;
use core::result::Result::{Err, Ok};
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::FetchErrorCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch_error::FetchError;
use moqtail::model::control::fetch_ok::FetchOk;
use moqtail::model::data::fetch_header::FetchHeader;
use moqtail::model::error::TerminationCode;
use moqtail::model::{common::reason_phrase::ReasonPhrase, control::constant::FetchType};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;
use tracing::{error, info, warn};

pub async fn handle(
  client: Arc<MOQTClient>,
  _control_stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::Fetch(m) => {
      info!("received Fetch message: {:?}", m);
      let fetch = *m;
      let request_id = fetch.clone().request_id;

      // check request id
      {
        let max_request_id = context
          .max_request_id
          .load(std::sync::atomic::Ordering::Relaxed);
        if request_id >= max_request_id {
          warn!(
            "request id ({}) is greater than max request id ({})",
            request_id, max_request_id
          );
          return Err(TerminationCode::TooManyRequests);
        }
      }

      let fn_ = async {
        if let Some(joining_fetch_props) = fetch.clone().joining_fetch_props {
          let sub_request_id = joining_fetch_props.joining_request_id;
          let sub_requests = client.subscribe_requests.read().await;
          // the original request id is the request id of the subscribe request that created the subscription
          let existing_sub = sub_requests
            .iter()
            .find(|e| e.1.original_request_id == sub_request_id);
          if existing_sub.is_none() {
            error!(
              "handle_fetch_messages | Joining fetch request id not found: {:?} {:?}",
              sub_request_id, sub_requests
            );
            // return Err(TerminationCode::InternalError);
            return (None, None, None);
          }
          let existing_sub = existing_sub.unwrap().1;

          let full_track_name = existing_sub
            .original_subscribe_request
            .get_full_track_name();
          let track_lock = context.track_manager.get_track(&full_track_name).await;

          if let Some(track_lock) = track_lock {
            let track = track_lock.read().await;
            let largest_location = track.largest_location.read().await;

            // TODO: validate the range
            if largest_location.group < joining_fetch_props.joining_start {
              error!(
                "handle_fetch_messages | Joining fetch start location is larger than the track's largest location: {:?} {:?}",
                largest_location, joining_fetch_props.joining_start
              );
              send_fetch_error(
                client.clone(),
                request_id,
                FetchErrorCode::InvalidRange,
                ReasonPhrase::try_new(String::from("Invalid range")).unwrap(),
              )
              .await;
              return (None, None, None);
            }

            let start_group = if fetch.fetch_type == FetchType::RelativeFetch {
              largest_location.group - joining_fetch_props.joining_start
            } else {
              joining_fetch_props.joining_start
            };

            let start_location = Location::new(start_group, 0);
            let end_location = Location::new(largest_location.group, 0);
            (
              Some(track_lock.clone()),
              Some(start_location),
              Some(end_location),
            )
          } else {
            (None, None, None)
          }
        } else {
          // standalone fetch
          let props = fetch.standalone_fetch_props.clone().unwrap();

          // let's see whether the track is in the cache
          let full_track_name = moqtail::model::data::full_track_name::FullTrackName {
            namespace: props.track_namespace.clone(),
            name: props.track_name.clone().into(),
          };
          let track = context.track_manager.get_track(&full_track_name).await;

          if let Some(track) = track {
            (
              Some(track),
              Some(props.start_location.clone()),
              Some(props.end_location.clone()),
            )
          } else {
            (None, None, None)
          }
        }
      };

      let (track, start_location, end_location) = fn_.await;

      // TODO: send fetch message to the publisher
      if track.is_none() {
        // TODO: send fetch message to the possible publishers
        // for now just return FETCH_ERROR
        send_fetch_error(
          client.clone(),
          request_id,
          FetchErrorCode::TrackDoesNotExist,
          ReasonPhrase::try_new(String::from("Track does not exist")).unwrap(),
        )
        .await;
        return Ok(());
      }

      info!(
        "handle_fetch_messages | Fetching objects from {:?} to {:?}",
        start_location.clone().unwrap(),
        end_location.clone().unwrap()
      );

      let track = track.unwrap();

      // Register a cancel channel for this fetch request
      let (cancel_tx, mut cancel_rx) = watch::channel(false);
      {
        let mut senders = client.fetch_cancel_senders.write().await;
        senders.insert(request_id, cancel_tx);
      }

      tokio::spawn(async move {
        // TODO: verify the range exist. Currently we just return what we have...
        let track_read = track.read().await;
        let mut object_rx = track_read
          .cache
          .read_objects(start_location.unwrap(), end_location.clone().unwrap())
          .await;

        let fetch_header = FetchHeader::new(request_id);
        let header_info = HeaderInfo::Fetch {
          header: fetch_header,
          fetch_request: fetch,
        };

        let stream_id = build_stream_id(track_read.track_alias, &header_info);

        let stream_fn = async move |client: Arc<MOQTClient>, stream_id: &StreamId| {
          let stream_result = client
            .open_stream(stream_id, fetch_header.serialize().unwrap(), 0)
            .await;

          match stream_result {
            Ok(send_stream) => Some(send_stream),
            Err(e) => {
              error!("handle_fetch_messages | Error opening stream: {:?}", e);
              None
            }
          }
        };

        let mut object_count = 0;
        let mut send_stream = None;
        let mut cancelled = false;
        loop {
          tokio::select! {
            event = object_rx.recv() => {
              match event {
                Some(event) => match event {
                  CacheConsumeEvent::NoObject => {
                    // there is no object found
                    break;
                  }
                  CacheConsumeEvent::EndLocation(end_location) => {
                    info!(
                      "handle_fetch_messages | sending fetch_ok | actual end_location: {:?}",
                      &end_location
                    );
                    // TODO: implement descending fetch
                    // TODO: end of track is correct?
                    let largest_location = track_read.largest_location.read().await;
                    let end_of_track = largest_location.group == end_location.group;
                    let fetch_ok =
                      FetchOk::new_ascending(request_id, end_of_track, end_location, vec![]);

                    client
                      .queue_message(ControlMessage::FetchOk(Box::new(fetch_ok)))
                      .await;
                  }
                  CacheConsumeEvent::Object(object) => {
                    if object_count == 0 {
                      info!("handle_fetch_messages | starting stream {:?}", &stream_id);
                      send_stream = match stream_fn(client.clone(), &stream_id).await {
                        Some(ss) => Some(ss),
                        None => {
                          // Clean up cancel sender before returning
                          client.fetch_cancel_senders.write().await.remove(&request_id);
                          return Err(TerminationCode::InternalError);
                        }
                      };
                    }
                    let object_id = object.object_id;
                    let is_sent = if let Err(e) = client
                      .write_stream_object(
                        &stream_id,
                        object_id,
                        object.serialize().unwrap(),
                        send_stream.as_ref().cloned(),
                      )
                      .await
                    {
                      error!(
                        "handle_fetch_messages | Error writing object to stream: {:?}",
                        e
                      );
                      false
                    } else {
                      true
                    };

                    if !is_sent {
                      // Clean up cancel sender before returning
                      client.fetch_cancel_senders.write().await.remove(&request_id);
                      return Err(TerminationCode::InternalError);
                    }

                    // Log fetch stream object if enabled
                    if context.server_config.enable_object_logging {
                      let sending_time = crate::server::utils::passed_time_since_start();
                      let fetch_object = moqtail::model::data::object::Object {
                        track_alias: track_read.track_alias,
                        location: moqtail::model::common::location::Location::new(
                          object.group_id,
                          object.object_id,
                        ),
                        publisher_priority: object.publisher_priority,
                        forwarding_preference:
                          moqtail::model::data::constant::ObjectForwardingPreference::Subgroup,
                        subgroup_id: Some(object.subgroup_id),
                        status: object
                          .object_status
                          .unwrap_or(moqtail::model::data::constant::ObjectStatus::Normal),
                        extensions: object.extension_headers.clone(),
                        payload: object.payload.clone(),
                      };
                      track_read
                        .object_logger
                        .log_fetch_object(
                          track_read.track_alias,
                          context.connection_id,
                          request_id,
                          &fetch_object,
                          is_sent,
                          sending_time,
                        )
                        .await;
                    }
                    info!(
                      "handle_fetch_messages | Wrote object to stream: {} object_id: {}",
                      &stream_id, object_id
                    );
                    object_count += 1;
                  }
                },
                None => {
                  warn!("handle_fetch_messages | No object.");
                  break;
                }
              }
            }
            _ = cancel_rx.changed() => {
              info!("handle_fetch_messages | Fetch cancelled for request_id: {}", request_id);
              cancelled = true;
              break;
            }
          }
        }

        if cancelled {
          // Close the stream promptly as per the spec
          if let Some(the_stream) = send_stream {
            if let Err(e) = the_stream.lock().await.shutdown().await {
              error!(
                "handle_fetch_messages | Error closing stream on cancel: {:?}",
                e
              );
            } else {
              info!(
                "handle_fetch_messages | closed fetch stream on cancel: {:?}",
                &stream_id
              );
            }
            client.remove_stream_by_stream_id(&stream_id).await;
          }
        } else if object_count == 0 {
          send_fetch_error(
            client.clone(),
            request_id,
            FetchErrorCode::NoObjects,
            ReasonPhrase::try_new(String::from("No objects available")).unwrap(),
          )
          .await;
        } else {
          // close the stream instantly
          if let Some(the_stream) = send_stream {
            // gracefully finish the stream here
            if let Err(e) = the_stream.lock().await.shutdown().await {
              error!("handle_fetch_messages | Error closing stream: {:?}", e);
              // return Err(TerminationCode::InternalError);
            } else {
              info!("finished fetch stream: {:?}", &stream_id);
            }
            client.remove_stream_by_stream_id(&stream_id).await;
            info!("removed stream from the map {}", stream_id);
          }
        }

        // Clean up cancel sender
        client
          .fetch_cancel_senders
          .write()
          .await
          .remove(&request_id);
        Ok(())
      });

      Ok(())
    }
    ControlMessage::FetchCancel(m) => {
      info!("received FetchCancel message: {:?}", m);
      let request_id = m.request_id;

      // Look up the cancel sender for this request and signal cancellation
      let cancel_tx = {
        let mut senders = client.fetch_cancel_senders.write().await;
        senders.remove(&request_id)
      };

      if let Some(tx) = cancel_tx {
        let _ = tx.send(true);
        info!(
          "handle_fetch_messages | Sent cancel signal for request_id: {}",
          request_id
        );
      } else {
        warn!(
          "handle_fetch_messages | FetchCancel received but no active fetch for request_id: {}",
          request_id
        );
      }

      Ok(())
    }
    ControlMessage::FetchOk(m) => {
      info!("received FetchOk message: {:?}", m);
      let msg = *m;

      // TODO: When the relay sends a fetch request to the publisher,
      // it will wait for Fetch OK. However this is not implemented yet.
      // Here is just a preliminary attempt for this, validating request id
      let requests = context.relay_fetch_requests.read().await;
      if !requests.contains_key(&msg.request_id) {
        error!("handle_fetch_messages | FetchOk | request_id does not exist");
        return Err(TerminationCode::InternalError);
      }

      Ok(())
    }
    _ => {
      // no-op
      Ok(())
    }
  }
}

async fn send_fetch_error(
  client: Arc<MOQTClient>,
  request_id: u64,
  error_code: FetchErrorCode,
  reason_phrase: ReasonPhrase,
) {
  let fetch_error = FetchError::new(request_id, error_code, reason_phrase);
  client
    .queue_message(ControlMessage::FetchError(Box::new(fetch_error)))
    .await;
}
