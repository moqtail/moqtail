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
use crate::server::session_context::{PendingRequest, SessionContext};
use core::result::Result;
use moqtail::model::error::TerminationCode;
use moqtail::model::{
  common::reason_phrase::ReasonPhrase, control::constant::RequestErrorCode,
  control::control_message::ControlMessage, control::request_error::RequestError,
  control::request_ok::RequestOk, control::subscribe::Subscribe,
  parameter::message_parameter::MessageParameter,
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
        let max_request_id = context
          .max_request_id
          .load(std::sync::atomic::Ordering::Relaxed);
        if request_id >= max_request_id {
          warn!("request id ({}) > max ({})", request_id, max_request_id);
          return Err(TerminationCode::TooManyRequests);
        }
      }

      // B. Check Local Track Existence
      if let Some(track_arc) = context.track_manager.get_track(&full_track_name).await {
        info!("track found: {:?}", full_track_name);
        let track = track_arc.read().await;
        let largest_location = track.largest_location.read().await;

        let params = vec![MessageParameter::new_largest_object(
          largest_location.clone(),
        )];

        let ok_msg = RequestOk::new(request_id, params);
        control_stream_handler
          .send(&ControlMessage::RequestOk(Box::new(ok_msg)))
          .await
          .unwrap();
        return Ok(());
      }

      // C. Find Upstream Publisher
      // TODO: send to every interested publisher
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
        let err = RequestError::new(
          request_id,
          RequestErrorCode::DoesNotExist,
          0, //TODO: Maybe decide on another retry interval?
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

      // E. Store Mapping
      // We convert TrackStatus -> Subscribe to fit it into 'SubscribeRequest' container
      let mut fake_params = vec![
        MessageParameter::new_forward(status_req.forward),
        MessageParameter::new_subscriber_priority(status_req.subscriber_priority),
        MessageParameter::new_subscription_filter(
          status_req.filter_type,
          status_req.start_location,
          status_req.end_group,
        ),
      ];

      fake_params.extend(status_req.subscribe_parameters.clone());

      let fake_sub = Subscribe {
        request_id: status_req.request_id,
        track_namespace: status_req.track_namespace,
        track_name: status_req.track_name,
        subscribe_parameters: fake_params,
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

      let mut map = context.relay_pending_requests.write().await;
      map.insert(relay_request_id, PendingRequest::TrackStatus(req_mapping));

      Ok(())
    }
    ControlMessage::RequestOk(m) => {
      info!("received RequestOk from Publisher: {:?}", m);
      let msg = *m;

      // A. Look up who asked for this and remove it from the map
      let mapping = {
        let mut map = context.relay_pending_requests.write().await;
        match map.remove(&msg.request_id) {
          Some(PendingRequest::TrackStatus(req)) => Some(req),
          Some(_) => {
            warn!(
              "Mismatched request type for TrackStatusOk: {}",
              msg.request_id
            );
            None
          }
          None => None,
        }
      };

      if let Some(req) = mapping {
        // C. Find the Client
        let manager = context.client_manager.read().await;
        if let Some(downstream_client) = manager.get(req.requested_by).await {
          // D. Construct Forwarded Message
          let forwarded_msg = RequestOk::new(req.original_request_id, msg.parameters);

          info!(
            "Forwarding RequestOk (for TrackStatus) to Client {}",
            req.requested_by
          );
          downstream_client
            .queue_message(ControlMessage::RequestOk(Box::new(forwarded_msg)))
            .await;
        } else {
          warn!("Downstream client {} disconnected", req.requested_by);
        }
      } else {
        // we shouldn't hit this normally
        debug!(
          "Received RequestOk for request ID {} (Not a TrackStatus relay)",
          msg.request_id
        );
      }
      Ok(())
    }
    ControlMessage::RequestError(m) => {
      info!("received TrackStatusError from Publisher: {:?}", m);
      let msg = *m;

      let mapping = {
        let mut map = context.relay_pending_requests.write().await;
        match map.remove(&msg.request_id) {
          Some(PendingRequest::TrackStatus(req)) => Some(req),
          Some(_) => {
            warn!(
              "Mismatched request type for TrackStatusError: {}",
              msg.request_id
            );
            None
          }
          None => None,
        }
      };

      if let Some(req) = mapping {
        let manager = context.client_manager.read().await;
        if let Some(downstream_client) = manager.get(req.requested_by).await {
          let forwarded_msg = RequestError::new(
            req.original_request_id, // <--- Restore ID
            msg.error_code,
            0, //TODO: Maybe decide on another retry interval?
            msg.reason_phrase,
          );

          info!("Forwarding TrackStatusError to Client {}", req.requested_by);
          downstream_client
            .queue_message(ControlMessage::RequestError(Box::new(forwarded_msg)))
            .await;
        }
      }
      Ok(())
    }

    ControlMessage::RequestUpdate(m) => {
      warn!(
        "Protocol Violation: Client attempted to update TrackStatus request ID {}",
        m.existing_request_id
      );
      Err(TerminationCode::ProtocolViolation)
    }

    _ => Ok(()),
  }
}
