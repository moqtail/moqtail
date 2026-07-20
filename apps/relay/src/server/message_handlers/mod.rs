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
  model::{control::control_message::ControlMessage, error::TerminationCode},
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
  ) -> Result<(), TerminationCode> {
    let handling_result = match &msg {
      ControlMessage::PublishNamespace(_) => {
        publish_namespace_handler::handle(client.clone(), stream_handler, msg, context.clone())
          .await
      }
      ControlMessage::SubscribeNamespace(_) => {
        warn!("SUBSCRIBE_NAMESPACE received on control stream — must use a dedicated bi-stream");
        Err(TerminationCode::ProtocolViolation)
      }
      ControlMessage::Subscribe(_) | ControlMessage::Switch(_) => {
        subscribe_handler::handle(client.clone(), stream_handler, msg, context.clone()).await
      }

      ControlMessage::TrackStatus(_) => {
        track_status_handler::handle(stream_handler, msg, context.clone()).await
      }
      ControlMessage::Fetch(_) => {
        fetch_handler::handle(client.clone(), stream_handler, msg, context.clone()).await
      }
      ControlMessage::Publish(_) | ControlMessage::PublishDone(_) => {
        publish_handler::handle(client.clone(), stream_handler, msg, context.clone()).await
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

      ControlMessage::RequestUpdate(m) => {
        let target_req_id = m.existing_request_id;

        enum Route {
          Fetch,
          Publish,
          PublishNamespace,
          Subscribe,
          SubscribeNamespace,
          TrackStatus,
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
          Some(PendingRequest::TrackStatus(_)) => Route::TrackStatus,
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
            fetch_handler::handle(client.clone(), stream_handler, msg, context.clone()).await
          }
          Route::Publish => {
            publish_handler::handle(client.clone(), stream_handler, msg, context.clone()).await
          }
          Route::PublishNamespace => {
            publish_namespace_handler::handle(client.clone(), stream_handler, msg, context.clone())
              .await
          }
          Route::Subscribe => {
            subscribe_handler::handle(client.clone(), stream_handler, msg, context.clone()).await
          }
          Route::SubscribeNamespace => {
            subscribe_namespace_handler::handle(
              client.clone(),
              stream_handler,
              msg,
              context.clone(),
            )
            .await
          }
          Route::TrackStatus => {
            track_status_handler::handle(stream_handler, msg, context.clone()).await
          }
          Route::NotFound => {
            // A REQUEST_UPDATE referencing an unknown request is disallowed: it
            // must travel on the request's own stream.
            warn!(
              "REQUEST_UPDATE for untracked request id {}; closing session",
              target_req_id
            );
            Err(TerminationCode::ProtocolViolation)
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
