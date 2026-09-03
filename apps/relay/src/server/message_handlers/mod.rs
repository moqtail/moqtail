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
use tracing::{debug, info, warn};

use crate::server::{
  client::MOQTClient,
  session_context::{PendingRequest, SessionContext},
};
use std::sync::Arc;
pub(crate) mod fetch_handler;
pub(crate) mod parameters;
mod publish_handler;
pub(crate) mod publish_namespace_handler;
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
      ControlMessage::Subscribe(_) => {
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

      // A response belongs on a request's own bidi stream, never on the control
      // stream. On a request stream it is the peer answering something the relay
      // asked there — REQUEST_UPDATE raising a publisher's Forward State is the
      // one that arrives here — and the request is already resolved by the time
      // it lands, so there is nothing left to do but let the stream continue.
      ControlMessage::RequestOk(_) | ControlMessage::RequestError(_) => match opening_request_id {
        Some(request_id) => {
          debug!(
            "{:?} answering relay request {} on its own stream",
            msg.get_type(),
            request_id
          );
          Ok(())
        }
        None => {
          warn!(
            "{:?} on the control stream; closing session",
            msg.get_type()
          );
          Err(TerminationCode::ProtocolViolation)
        }
      },

      ControlMessage::SubscribeOk(_) | ControlMessage::FetchOk(_) => {
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
          /// SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS are updated the same way.
          PrefixSubscription,
          NotFound,
        }

        // Map a PendingRequest to a Route
        let determine_route = |req: Option<&PendingRequest>| match req {
          Some(PendingRequest::Fetch(_)) => Route::Fetch,
          Some(PendingRequest::Publish { .. }) => Route::Publish,
          Some(PendingRequest::PublishNamespace { .. }) => Route::PublishNamespace,
          Some(PendingRequest::Subscribe(_)) => Route::Subscribe,
          Some(PendingRequest::SubscribeNamespace { .. }) => Route::PrefixSubscription,
          Some(PendingRequest::SubscribeTracks { .. }) => Route::PrefixSubscription,
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
          Route::PrefixSubscription => {
            crate::server::prefix_subscription::handle(
              client.clone(),
              stream_handler,
              msg,
              context.clone(),
              opening_request_id,
            )
            .await
          }
          Route::NotFound => {
            // A REQUEST_UPDATE must be answered with exactly one REQUEST_OK or
            // REQUEST_ERROR, so an update naming a request the relay holds no
            // updatable record of is rejected rather than terminating.
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
