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

use crate::server::session::Session;
use crate::server::session_context::SessionContext;
use core::result::Result;
use moqtail::model::error::TerminationCode;
use moqtail::model::{
  common::reason_phrase::ReasonPhrase,
  control::constant::SubscribeErrorCode,
  control::control_message::ControlMessage,
  control::subscribe::Subscribe, // Needed for the storage hack
  control::track_status_error::TrackStatusError,
  control::track_status_ok::TrackStatusOk,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::SubscribeRequest;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub async fn handle(
  control_stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::TrackStatus(m) => {
      info!("received TrackStatus message: {:?}", m);
      let status_req = *m;
      let track_namespace = status_req.track_namespace.clone();
      let request_id = status_req.request_id;
      let full_track_name = status_req.get_full_track_name();

      // A. Check Max Request ID
      {
        let max_request_id = context.max_request_id.read().await;
        if request_id >= *max_request_id {
          warn!("request id ({}) > max ({})", request_id, max_request_id);
          return Err(TerminationCode::TooManyRequests);
        }
      }

      // B. Check Local Track Existence (Short Circuit)
      // If Relay already has this track, we can answer "OK" immediately.
      if context.track_manager.has_track(&full_track_name).await {
        info!("Track found locally on Relay. Sending TrackStatusOk directly.");

        let ok_msg = TrackStatusOk::new_ascending_with_content(request_id, 0, 0, None, None);

        control_stream_handler.send_impl(&ok_msg).await.unwrap();
        return Ok(());
      }

      // C. Find Upstream Publisher
      let publisher = {
        debug!("Finding publisher for TrackStatus...");
        let m = context.client_manager.read().await;
        match m.get_publisher_by_full_track_name(&full_track_name).await {
          Some(p) => Some(p),
          None => {
            m.get_publisher_by_announced_track_namespace(&track_namespace)
              .await
          }
        }
      };

      let publisher = if let Some(p) = publisher {
        p.clone()
      } else {
        info!("No publisher found for {:?}", track_namespace);
        let err = TrackStatusError::new(
          request_id,
          SubscribeErrorCode::TrackDoesNotExist,
          ReasonPhrase::try_new("No publisher found".to_string()).unwrap(),
        );
        control_stream_handler.send_impl(&err).await.unwrap();
        return Ok(());
      };

      // D. Forward to Publisher
      info!(
        "Forwarding TrackStatus to Publisher: {}",
        publisher.connection_id
      );

      let mut new_req = status_req.clone();
      let relay_request_id =
        Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;
      new_req.request_id = relay_request_id;

      publisher
        .queue_message(ControlMessage::TrackStatus(Box::new(new_req.clone())))
        .await;

      // E. Store Mapping (The Storage Hack)
      // We convert TrackStatus -> Subscribe to fit it into 'SubscribeRequest' container
      // This is safe because fields are identical.
      let fake_sub = Subscribe {
        request_id: status_req.request_id,
        track_namespace: status_req.track_namespace,
        track_name: status_req.track_name,
        subscriber_priority: status_req.subscriber_priority,
        group_order: status_req.group_order,
        forward: status_req.forward,
        filter_type: status_req.filter_type,
        start_location: status_req.start_location,
        end_group: status_req.end_group,
        subscribe_parameters: status_req.subscribe_parameters,
      };

      // We also need a fake "new_sub" for the relay-side mapping
      let mut fake_new_sub = fake_sub.clone();
      fake_new_sub.request_id = relay_request_id;

      let req_mapping = SubscribeRequest::new(
        request_id,
        context.connection_id,
        fake_sub,
        Some(fake_new_sub),
      );

      let mut map = context.relay_track_status_requests.write().await;
      map.insert(relay_request_id, req_mapping);

      Ok(())
    }

    // =======================================================================
    // 2. HANDLE OK RESPONSE (Publisher -> Relay -> Client)
    // =======================================================================
    ControlMessage::TrackStatusOk(m) => {
      info!("received TrackStatusOk from Publisher: {:?}", m);
      let msg = *m;

      // A. Look up who asked for this
      let mapping = {
        let map = context.relay_track_status_requests.read().await;
        map.get(&msg.request_id).cloned()
      };

      if let Some(req) = mapping {
        // B. Find the Client
        let manager = context.client_manager.read().await;
        if let Some(downstream_client) = manager.get(req.requested_by).await {
          // C. Construct Forwarded Message
          // We must restore the ORIGINAL request ID that the client sent us
          let forwarded_msg = TrackStatusOk::new_ascending_with_content(
            req.original_request_id,
            msg.track_alias,
            msg.expires,
            msg.largest_location,
            msg.subscribe_parameters,
          );

          info!("Forwarding TrackStatusOk to Client {}", req.requested_by);
          downstream_client
            .queue_message(ControlMessage::TrackStatusOk(Box::new(forwarded_msg)))
            .await;
        } else {
          warn!("Downstream client {} disconnected", req.requested_by);
        }
      } else {
        warn!(
          "Received TrackStatusOk for unknown request ID: {}",
          msg.request_id
        );
      }
      Ok(())
    }

    // =======================================================================
    // 3. HANDLE ERROR RESPONSE (Publisher -> Relay -> Client)
    // =======================================================================
    ControlMessage::TrackStatusError(m) => {
      info!("received TrackStatusError from Publisher: {:?}", m);
      let msg = *m;

      let mapping = {
        let map = context.relay_track_status_requests.read().await;
        map.get(&msg.request_id).cloned()
      };

      if let Some(req) = mapping {
        let manager = context.client_manager.read().await;
        if let Some(downstream_client) = manager.get(req.requested_by).await {
          let forwarded_msg = TrackStatusError::new(
            req.original_request_id, // <--- Restore ID
            msg.error_code,
            msg.reason_phrase,
          );

          info!("Forwarding TrackStatusError to Client {}", req.requested_by);
          downstream_client
            .queue_message(ControlMessage::TrackStatusError(Box::new(forwarded_msg)))
            .await;
        }
      }
      Ok(())
    }

    _ => Ok(()),
  }
}
