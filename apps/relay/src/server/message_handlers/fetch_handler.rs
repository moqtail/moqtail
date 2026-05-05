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
use crate::server::session_context::{PendingRequest, SessionContext, UpstreamFetchEvent};
use crate::server::stream_id::StreamId;
use crate::server::utils::build_stream_id;
use core::result::Result::{Err, Ok};
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::RequestErrorCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::Fetch;
use moqtail::model::control::fetch_ok::FetchOk;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::data::fetch_header::FetchHeader;
use moqtail::model::error::TerminationCode;
use moqtail::model::{common::reason_phrase::ReasonPhrase, control::constant::FetchType};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{FetchRequest, HeaderInfo};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

const UPSTREAM_FETCH_CHANNEL_CAPACITY: usize = 64;

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

      {
        let req = FetchRequest {
          original_request_id: request_id,
          requested_by: client.connection_id,
          fetch_request: fetch.clone(),
          track_alias: 0,
        };

        client
          .incoming_fetch_requests
          .write()
          .await
          .insert(request_id, req.clone());
        client
          .inbound_requests
          .write()
          .await
          .insert(request_id, PendingRequest::Fetch(req));
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
              send_request_error(
                client.clone(),
                request_id,
                RequestErrorCode::InvalidRange,
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
            name: props.track_name.clone(),
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
        // for now just return REQUEST_ERROR
        send_request_error(
          client.clone(),
          request_id,
          RequestErrorCode::DoesNotExist,
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
      {
        let track_read = track.read().await;

        let mut inbound = client.inbound_requests.write().await;
        if let Some(PendingRequest::Fetch(req)) = inbound.get_mut(&request_id) {
          req.track_alias = track_read.relay_track_id;
        }
        let mut fetches = client.incoming_fetch_requests.write().await;
        if let Some(req) = fetches.get_mut(&request_id) {
          req.track_alias = track_read.relay_track_id;
        }
      }

      // Register a cancel channel for this fetch request
      let (cancel_tx, cancel_rx) = watch::channel(false);
      {
        let mut senders = client.fetch_cancel_senders.write().await;
        senders.insert(request_id, cancel_tx);
      }

      tokio::spawn(async move {
        let track_read = track.read().await;
        let start_location = start_location.unwrap();
        let end_location = end_location.unwrap();

        let fetch_header = FetchHeader::new(request_id);
        let header_info = HeaderInfo::Fetch {
          header: fetch_header,
          fetch_request: fetch.clone(),
        };

        let stream_id = build_stream_id(track_read.relay_track_id, &header_info);

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
        let mut upstream_gap_count: u64 = 0;
        let mut send_stream = None;
        let mut cancelled = false;
        let mut fetch_prev_ctx: Option<moqtail::model::data::fetch_object::FetchObjectContext> =
          None;
        let mut group_id = start_location.group;

        while group_id <= end_location.group {
          if *cancel_rx.borrow() {
            info!(
              "handle_fetch_messages | Fetch cancelled for request_id: {}",
              request_id
            );
            cancelled = true;
            break;
          }

          if let Some(group_objects) = track_read.cache.get_group(group_id).await {
            // === CACHE HIT ===
            let objects = group_objects.read().await;
            for object in objects.iter() {
              if group_id == start_location.group && object.object_id < start_location.object {
                continue;
              }
              if group_id == end_location.group && object.object_id >= end_location.object {
                break;
              }

              if object_count == 0 {
                send_stream = match stream_fn(client.clone(), &stream_id).await {
                  Some(ss) => Some(ss),
                  None => {
                    client
                      .fetch_cancel_senders
                      .write()
                      .await
                      .remove(&request_id);
                    return Err(TerminationCode::InternalError);
                  }
                };
              }

              let fetch_obj =
                moqtail::model::data::fetch_object::FetchObject::Object(object.clone());
              let serialized = fetch_obj.serialize(fetch_prev_ctx.as_ref()).unwrap();
              fetch_prev_ctx = fetch_obj.context();

              if let Err(e) = client
                .write_stream_object(
                  &stream_id,
                  object.object_id,
                  serialized,
                  send_stream.as_ref().cloned(),
                )
                .await
              {
                error!(
                  "handle_fetch_messages | Error writing object to stream: {:?}",
                  e
                );
                client
                  .fetch_cancel_senders
                  .write()
                  .await
                  .remove(&request_id);
                return Err(TerminationCode::InternalError);
              }

              if context.server_config.enable_object_logging {
                let sending_time = crate::server::utils::passed_time_since_start();
                if let Ok(fetch_object) = moqtail::model::data::object::Object::try_from_fetch(
                  object.clone(),
                  track_read.relay_track_id,
                ) {
                  track_read
                    .object_logger
                    .log_fetch_object(
                      track_read.relay_track_id,
                      context.connection_id,
                      request_id,
                      &fetch_object,
                      true,
                      sending_time,
                    )
                    .await;
                }
              }

              object_count += 1;
            }
            group_id += 1;
          } else {
            // === CACHE MISS ===
            // Scan ahead to find contiguous gap [gap_start .. gap_end]
            let gap_start: u64 = group_id;
            let mut gap_end = group_id;
            while gap_end < end_location.group
              && track_read.cache.get_group(gap_end + 1).await.is_none()
            {
              gap_end += 1;
            }

            let max_upstream_fetch_gaps = context.server_config.max_upstream_fetch_gaps;
            if upstream_gap_count >= max_upstream_fetch_gaps {
              warn!(
                "handle_fetch_delivery | Reached max upstream fetch gap limit ({}), skipping gap at group {}",
                max_upstream_fetch_gaps, gap_start
              );
              group_id = gap_end + 1;
              continue;
            }
            upstream_gap_count += 1;

            // Issue upstream fetch for the gap
            let upstream_rx = send_upstream_fetch_for_range(
              &client,
              &context,
              &track_read,
              &fetch,
              gap_start,
              gap_end,
            )
            .await;

            if let Some(mut rx) = upstream_rx {
              let timeout = context.server_config.upstream_fetch_timeout;
              loop {
                match tokio::time::timeout(timeout, rx.recv()).await {
                  Ok(Some(UpstreamFetchEvent::Object(object))) => {
                    if object_count == 0 {
                      send_stream = match stream_fn(client.clone(), &stream_id).await {
                        Some(ss) => Some(ss),
                        None => {
                          client
                            .fetch_cancel_senders
                            .write()
                            .await
                            .remove(&request_id);
                          return Err(TerminationCode::InternalError);
                        }
                      };
                    }

                    let fetch_obj =
                      moqtail::model::data::fetch_object::FetchObject::Object(object.clone());
                    let serialized = fetch_obj.serialize(fetch_prev_ctx.as_ref()).unwrap();
                    fetch_prev_ctx = fetch_obj.context();

                    if let Err(e) = client
                      .write_stream_object(
                        &stream_id,
                        object.object_id,
                        serialized,
                        send_stream.as_ref().cloned(),
                      )
                      .await
                    {
                      error!(
                        "handle_fetch_messages | Error writing upstream object to stream: {:?}",
                        e
                      );
                      client
                        .fetch_cancel_senders
                        .write()
                        .await
                        .remove(&request_id);
                      return Err(TerminationCode::InternalError);
                    }

                    if context.server_config.enable_object_logging {
                      let sending_time = crate::server::utils::passed_time_since_start();
                      if let Ok(fetch_object) = moqtail::model::data::object::Object::try_from_fetch(
                        object.clone(),
                        track_read.relay_track_id,
                      ) {
                        track_read
                          .object_logger
                          .log_fetch_object(
                            track_read.relay_track_id,
                            context.connection_id,
                            request_id,
                            &fetch_object,
                            true,
                            sending_time,
                          )
                          .await;
                      }
                    }

                    object_count += 1;
                  }
                  Ok(Some(UpstreamFetchEvent::StreamClosed)) => {
                    break;
                  }
                  Ok(Some(UpstreamFetchEvent::Error(e))) => {
                    warn!(
                      "handle_fetch_messages | Upstream fetch error for gap [{}, {}]: {}",
                      gap_start, gap_end, e
                    );
                    break;
                  }
                  Ok(None) => {
                    break;
                  }
                  Err(_) => {
                    warn!(
                      "handle_fetch_messages | Upstream fetch timed out for gap [{}, {}]",
                      gap_start, gap_end
                    );
                    context
                      .upstream_fetch_senders
                      .write()
                      .await
                      .remove(&gap_start);
                    break;
                  }
                }
              }
            }

            group_id = gap_end + 1;
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
          // Draft 16: If range is valid but empty, send FETCH_OK + Empty Stream with FIN.
          info!(
            "handle_fetch_messages | Empty range for request_id: {}. Sending FETCH_OK and empty stream.",
            request_id
          );

          // 1. Send FETCH_OK
          let end_loc = start_location.clone();
          let fetch_ok = FetchOk::new(request_id, false, end_loc, vec![], vec![]);

          client
            .queue_message(ControlMessage::FetchOk(Box::new(fetch_ok)))
            .await;

          // 2. Open Stream, Write Header, and Close (FIN)
          if let Some(the_stream) = stream_fn(client.clone(), &stream_id).await {
            let mut stream_lock = the_stream.lock().await;
            if let Err(e) = stream_lock.shutdown().await {
              error!(
                "handle_fetch_messages | Error closing empty fetch stream: {:?}",
                e
              );
            }
            client.remove_stream_by_stream_id(&stream_id).await;
          }
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

        // Clean up the unified map since the fetch is done
        client.inbound_requests.write().await.remove(&request_id);
        client
          .incoming_fetch_requests
          .write()
          .await
          .remove(&request_id);

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

      // Remove the fetch request from the client maps
      {
        client.inbound_requests.write().await.remove(&request_id);
        client
          .incoming_fetch_requests
          .write()
          .await
          .remove(&request_id);
        debug!(
          "Removed FETCH request {} from client maps after Cancel",
          request_id
        );
      }

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
      let pending_request = {
        let map = context.relay_pending_requests.read().await;
        map.get(&msg.request_id).cloned()
      };

      if pending_request.is_none() {
        error!("handle_fetch_messages | FetchOk | request_id does not exist in pending registry");
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

/// Send an upstream Fetch to the publisher for a cache gap [gap_start, gap_end].
/// Returns an mpsc::Receiver through which upstream objects will be forwarded.
#[allow(dead_code)]
async fn send_upstream_fetch_for_range(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  track_read: &crate::server::track::Track,
  original_fetch: &Fetch,
  gap_start: u64,
  gap_end: u64,
) -> Option<mpsc::Receiver<UpstreamFetchEvent>> {
  let publisher = {
    let m = context.client_manager.read().await;
    match m
      .get_publisher_by_full_track_name(&track_read.full_track_name)
      .await
    {
      Some(p) => Some(p),
      None => {
        m.get_publisher_by_announced_track_namespace(&track_read.full_track_name.namespace)
          .await
      }
    }
  };
  let publisher = match publisher {
    Some(p) => p,
    None => {
      info!(
        "send_upstream_fetch_for_range | No publisher found for {:?}",
        &track_read.full_track_name
      );
      return None;
    }
  };

  let relay_request_id = crate::server::session::Session::get_next_relay_request_id(
    context.relay_next_request_id.clone(),
  )
  .await;

  let standalone_props = moqtail::model::control::fetch::StandaloneFetchProps {
    track_namespace: track_read.full_track_name.namespace.clone(),
    track_name: track_read.full_track_name.name.clone(),
    start_location: Location::new(gap_start, 0),
    end_location: Location::new(gap_end, 0),
  };
  let upstream_fetch = Fetch::new_standalone(
    relay_request_id,
    standalone_props,
    original_fetch.parameters.clone(),
  );

  let (upstream_tx, upstream_rx) = mpsc::channel(UPSTREAM_FETCH_CHANNEL_CAPACITY);

  {
    // Use the publisher's track alias so handle_uni_stream can resolve the track on the response.
    let publisher_alias = track_read
      .publisher_aliases
      .read()
      .await
      .get(&publisher.connection_id)
      .copied()
      .unwrap_or(0);
    let req = FetchRequest {
      original_request_id: relay_request_id,
      requested_by: client.connection_id,
      fetch_request: upstream_fetch.clone(),
      track_alias: publisher_alias,
    };
    publisher
      .outgoing_fetch_requests
      .write()
      .await
      .insert(relay_request_id, req.clone());
    context
      .relay_pending_requests
      .write()
      .await
      .insert(relay_request_id, PendingRequest::Fetch(req));
    context
      .upstream_fetch_senders
      .write()
      .await
      .insert(relay_request_id, upstream_tx);
  }

  publisher
    .queue_message(ControlMessage::Fetch(Box::new(upstream_fetch)))
    .await;

  Some(upstream_rx)
}

async fn send_request_error(
  client: Arc<MOQTClient>,
  request_id: u64,
  error_code: RequestErrorCode,
  reason_phrase: ReasonPhrase,
) {
  // TODO: Implement this later.
  // Draft 16 requires a retry interval. Setting to 0 (no retries) for now.
  let retry_interval = 0;
  let request_error = RequestError::new(request_id, error_code, retry_interval, reason_phrase);

  client
    .queue_message(ControlMessage::RequestError(Box::new(request_error)))
    .await;
  // Remove the request from the client maps on error
  client.inbound_requests.write().await.remove(&request_id);
  client
    .incoming_fetch_requests
    .write()
    .await
    .remove(&request_id);
}
