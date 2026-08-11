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

use bytes::Bytes;
use moqtail::{
  model::{
    common::reason_phrase::ReasonPhrase,
    control::{control_message::ControlMessage, request_error::RequestError},
    error::{RequestErrorCode, TerminationCode},
  },
  transport::control_stream_handler::ControlStreamHandler,
};
use tracing::{info, warn};

use crate::server::{
  client::MOQTClient,
  session_context::{PendingRequest, SessionContext},
};
use std::sync::Arc;
pub(crate) mod fetch_handler;
mod publish_handler;
mod publish_namespace_handler;
pub(crate) mod subscribe_handler;
pub(crate) mod subscribe_namespace_handler;
pub(crate) mod subscribe_tracks_handler;
mod track_status_handler;
use super::utils;

pub struct MessageHandler {}

impl MessageHandler {
  pub async fn handle(
    client: Arc<MOQTClient>,
    stream_handler: &mut ControlStreamHandler,
    msg: ControlMessage,
    context: Arc<SessionContext>,
    // Request ID of the First-marked message that opened this request stream,
    // or None on the control stream.
    opening_request_id: Option<u64>,
  ) -> Result<(), TerminationCode> {
    let handling_result = match &msg {
      ControlMessage::PublishNamespace(_) => {
        publish_namespace_handler::handle(
          client.clone(),
          stream_handler,
          msg,
          context.clone(),
          opening_request_id,
        )
        .await
      }
      ControlMessage::SubscribeNamespace(_) => {
        warn!("SUBSCRIBE_NAMESPACE received on control stream — must use a dedicated bi-stream");
        Err(TerminationCode::ProtocolViolation)
      }
      ControlMessage::Subscribe(_) | ControlMessage::Switch(_) => {
        subscribe_handler::handle(
          client.clone(),
          stream_handler,
          msg,
          context.clone(),
          opening_request_id,
        )
        .await
      }

      ControlMessage::TrackStatus(_) => {
        track_status_handler::handle(stream_handler, msg, context.clone()).await
      }
      ControlMessage::Fetch(_) => {
        fetch_handler::handle(
          client.clone(),
          stream_handler,
          msg,
          context.clone(),
          opening_request_id,
        )
        .await
      }
      ControlMessage::Publish(_) | ControlMessage::PublishDone(_) => {
        publish_handler::handle(
          client.clone(),
          stream_handler,
          msg,
          context.clone(),
          opening_request_id,
        )
        .await
      }

      // A response is read on the request's own bidi stream; reaching the control
      // stream is a disallowed action.
      ControlMessage::RequestOk(_)
      | ControlMessage::RequestError(_)
      | ControlMessage::SubscribeOk(_)
      | ControlMessage::FetchOk(_) => {
        warn!(
          "{:?} on the control stream; closing session",
          msg.get_type()
        );
        Err(TerminationCode::ProtocolViolation)
      }

      ControlMessage::RequestUpdate(_) => {
        let Some(target_req_id) = opening_request_id else {
          warn!("REQUEST_UPDATE on the control stream; closing session");
          return Err(TerminationCode::ProtocolViolation);
        };

        enum Route {
          Fetch,
          Publish,
          PublishNamespace,
          Subscribe,
          SubscribeNamespace,
          NotFound,
        }

        // Map a PendingRequest to a Route
        let determine_route = |req: Option<&PendingRequest>| match req {
          Some(PendingRequest::Fetch(_)) => Route::Fetch,
          Some(PendingRequest::Publish { .. }) => Route::Publish,
          Some(PendingRequest::PublishNamespace { .. }) => Route::PublishNamespace,
          Some(PendingRequest::Subscribe(_)) => Route::Subscribe,
          Some(PendingRequest::SubscribeNamespace { .. }) => Route::SubscribeNamespace,
          Some(PendingRequest::SubscribeTracks { .. }) => Route::SubscribeNamespace,
          Some(PendingRequest::RequestUpdate { .. }) => Route::NotFound,
          None => Route::NotFound,
        };

        let route = {
          let map = client.inbound_requests.read().await;
          determine_route(map.get(&target_req_id))
        };

        // Route to the appropriate handler (defined only once!)
        match route {
          Route::Fetch => {
            fetch_handler::handle(
              client.clone(),
              stream_handler,
              msg,
              context.clone(),
              opening_request_id,
            )
            .await
          }
          Route::Publish => {
            publish_handler::handle(
              client.clone(),
              stream_handler,
              msg,
              context.clone(),
              opening_request_id,
            )
            .await
          }
          Route::PublishNamespace => {
            publish_namespace_handler::handle(
              client.clone(),
              stream_handler,
              msg,
              context.clone(),
              opening_request_id,
            )
            .await
          }
          Route::Subscribe => {
            subscribe_handler::handle(
              client.clone(),
              stream_handler,
              msg,
              context.clone(),
              opening_request_id,
            )
            .await
          }
          Route::SubscribeNamespace => {
            subscribe_namespace_handler::handle(
              client.clone(),
              stream_handler,
              msg,
              context.clone(),
              opening_request_id,
            )
            .await
          }
          Route::NotFound => {
            // Draft-18 10.9: a REQUEST_UPDATE must be answered with exactly one
            // REQUEST_OK or REQUEST_ERROR, so an update naming a request the relay
            // holds no updatable record of is rejected rather than terminating.
            warn!("REQUEST_UPDATE for untracked request id {}", target_req_id);
            let err = RequestError::new(
              RequestErrorCode::NotSupported,
              0,
              ReasonPhrase::try_new("request does not support REQUEST_UPDATE".to_string()).unwrap(),
            );
            stream_handler
              .send(&ControlMessage::RequestError(Box::new(err)))
              .await
          }
        }
      }

      // Catch-all for any unhandled control messages
      m => {
        info!("unhandled message received");
        if let Ok(a) = m.serialize() {
          let buf = Bytes::from_iter(a);
          utils::print_bytes(&buf);
        }
        Ok(())
      }
    };

    if let Err(termination_code) = handling_result {
      Err(termination_code)
    } else {
      Ok(())
    }
  }
}
