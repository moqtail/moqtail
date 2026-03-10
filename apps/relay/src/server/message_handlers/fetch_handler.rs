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

const MAX_UPSTREAM_FETCH_GAPS: u64 = 10;

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

      tokio::spawn(handle_fetch_delivery(
        client, context, track, fetch, request_id,
        start_location, end_location, cancel_rx,
      ));

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
    // Handle FetchError from upstream publisher
    ControlMessage::FetchError(m) => {
      let upstream_request_id = m.request_id;
      // Look up upstream_fetch_senders for this request_id.
      // If found, send UpstreamFetchEvent::Error through the channel.
      // Clean up relay_fetch_requests and upstream_fetch_senders.
      Ok(())
    }
    _ => Ok(()),
  }
}

/// Refactored delivery loop.
/// Iterates groups in [start, end] sequentially. For each group:
///   - Cache hit  → serve objects directly from cache
///   - Cache miss → scan ahead to find gap extent, call send_upstream_fetch_for_range(),
///                   receive objects via mpsc channel, deliver them to client
async fn handle_fetch_delivery(
  client: Arc<MOQTClient>,
  context: Arc<SessionContext>,
  track: Arc<tokio::sync::RwLock<crate::server::track::Track>>,
  fetch: Fetch,
  request_id: u64,
  start_location: Location,
  end_location: Location,
  mut cancel_rx: watch::Receiver<bool>,
) -> Result<(), TerminationCode> {
  let track_read = track.read().await;
  let fetch_header = FetchHeader::new(request_id);
  let header_info = HeaderInfo::Fetch { header: fetch_header, fetch_request: fetch.clone() };
  let stream_id = build_stream_id(track_read.track_alias, &header_info);
  let mut object_count: u64 = 0;
  let mut send_stream = None;
  let mut upstream_gap_count: u64 = 0;

  // Send FetchOk early on the control stream
  // ... build FetchOk with end_location, queue on client ...

  let mut group_id = start_location.group;

  while group_id <= end_location.group {
    // Check cancel_rx.has_changed() — break if cancelled

    if let Some(group_objects) = track_read.cache.get_group(group_id).await {
      // === CACHE HIT ===
      // Iterate objects in group, apply start/end filtering,
      // call deliver_object() for each (opens stream lazily on first object)
      group_id += 1;
    } else {
      // === CACHE MISS ===
      // Scan ahead to find contiguous gap [gap_start .. gap_end]
      let gap_start: u64 = group_id;
      let mut gap_end = group_id;
      // ... while next groups also missing, extend gap_end ...

      if upstream_gap_count >= MAX_UPSTREAM_FETCH_GAPS {
        warn!(
          "handle_fetch_delivery | Reached max upstream fetch gap limit ({}), skipping gap at group {}",
          MAX_UPSTREAM_FETCH_GAPS, gap_start
        );
        group_id = gap_end + 1;
        continue;
      }
      upstream_gap_count += 1;

      // Issue upstream fetch for the gap
      let upstream_rx = send_upstream_fetch_for_range(
        &client, &context, &track_read, &fetch, gap_start, gap_end,
      ).await;

      if let Some(mut rx) = upstream_rx {
        // This is the receiver loop. It will be responsible for
        // sending objects to the client.
        // We'll await objects from upstream_fetch_senders[relay_request_id]
        // and send them to the client, through deliver_object().
      }

      group_id = gap_end + 1;
    }
  }

  // Finish: shutdown stream, or send FETCH_ERROR if object_count == 0
  // Clean up cancel sender
  Ok(())
}

/// Deliver a single FetchObject to the downstream client.
/// Opens the unidirectional stream lazily on the first object.
async fn deliver_object(
  client: &Arc<MOQTClient>,
  stream_id: &StreamId,
  object: &FetchObject,
  object_count: &mut u64,
  send_stream: &mut Option<Arc<tokio::sync::Mutex<wtransport::SendStream>>>,
  fetch_header: &FetchHeader,
) -> bool {
  // If object_count == 0: client.open_stream(stream_id, fetch_header bytes, priority=0)
  // client.write_stream_object(stream_id, object_id, object.serialize(), send_stream)
  // Increment object_count
  // Return true on success, false on write failure
  true
}

/// NEW: Send an upstream Fetch to the publisher for a cache gap [gap_start, gap_end].
/// Returns an mpsc::Receiver through which upstream objects will be forwarded.
async fn send_upstream_fetch_for_range(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  track_read: &crate::server::track::Track,
  original_fetch: &Fetch,
  gap_start: u64,
  gap_end: u64,
) -> Option<mpsc::Receiver<UpstreamFetchEvent>> {
  // 1. Find publisher via client_manager (by full_track_name or announced namespace)
  //    Return None if no publisher found

  // 2. Allocate relay_request_id (odd, via Session::get_next_relay_request_id)

  // 3. Build upstream Fetch::new_standalone for range [gap_start, gap_end]

  // 4. Create mpsc::channel(64) → (upstream_tx, upstream_rx)

  // 5. Register:
  //    - publisher.fetch_requests[relay_request_id] = FetchRequest { ... }
  //    - context.relay_fetch_requests[relay_request_id] = same
  //    - context.upstream_fetch_senders[relay_request_id] = upstream_tx

  // 6. publisher.queue_message(ControlMessage::Fetch(upstream_fetch))

  // Some(upstream_rx)
  todo!()
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
