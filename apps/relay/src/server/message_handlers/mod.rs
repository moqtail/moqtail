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
use std::sync::{Arc, atomic::Ordering};
mod fetch_handler;
mod max_request_id_handler;
mod publish_handler;
mod publish_namespace_handler;
mod subscribe_handler;
mod subscribe_namespace_handler;
mod track_status_handler;
use super::utils;

pub struct MessageHandler {}

impl MessageHandler {
  pub async fn handle(
    client: Arc<MOQTClient>,
    control_stream_handler: &mut ControlStreamHandler,
    msg: ControlMessage,
    context: Arc<SessionContext>,
  ) -> Result<(), TerminationCode> {
    // Check request ID if the message is a request
    let request_id = match &msg {
      ControlMessage::PublishNamespace(msg) => Some(msg.request_id),
      ControlMessage::Publish(msg) => Some(msg.request_id),
      ControlMessage::Fetch(msg) => Some(msg.request_id),
      ControlMessage::Subscribe(msg) => Some(msg.request_id),
      ControlMessage::SubscribeUpdate(msg) => Some(msg.request_id),
      ControlMessage::TrackStatus(msg) => Some(msg.request_id),
      ControlMessage::SubscribeNamespace(msg) => Some(msg.request_id),
      ControlMessage::Switch(msg) => Some(msg.request_id),
      _ => None,
    };

    if let Some(request_id) = request_id {
      let max_request_id = context.max_request_id.load(Ordering::Relaxed);
      if request_id >= max_request_id {
        warn!(
          "request id ({}) is greater than max request id ({})",
          request_id, max_request_id
        );
        return Err(TerminationCode::TooManyRequests);
      }
    }

    let handling_result = match &msg {
      ControlMessage::PublishNamespace(_) => {
        publish_namespace_handler::handle(
          client.clone(),
          control_stream_handler,
          msg,
          context.clone(),
        )
        .await
      }
      ControlMessage::SubscribeNamespace(_) => {
        subscribe_namespace_handler::handle(
          client.clone(),
          control_stream_handler,
          msg,
          context.clone(),
        )
        .await
      }
      ControlMessage::MaxRequestId(_) => {
        max_request_id_handler::handle(client.clone(), control_stream_handler, msg, context.clone())
          .await
      }
      ControlMessage::Subscribe(_)
      | ControlMessage::SubscribeOk(_)
      | ControlMessage::SubscribeUpdate(_)
      | ControlMessage::Unsubscribe(_)
      | ControlMessage::Switch(_) => {
        subscribe_handler::handle(client.clone(), control_stream_handler, msg, context.clone())
          .await
      }
      
      ControlMessage::TrackStatus(_) => {
        track_status_handler::handle(control_stream_handler, msg, context.clone()).await
      }
      ControlMessage::RequestOk(ok_msg) => {
        let req_id = ok_msg.request_id;

        enum Route {
          TrackStatus,
          SubscribeNamespace,
          PublishNamespace,
          Unhandled,
          NotFound,
        }

        // 1. Peek at the unified map to determine the routing destination
        let route = {
          let map = context.relay_pending_requests.read().await;
          match map.get(&req_id) {
            Some(PendingRequest::TrackStatus(_)) => Route::TrackStatus,
            Some(PendingRequest::SubscribeNamespace { .. }) => Route::SubscribeNamespace,
            Some(PendingRequest::PublishNamespace { .. }) => Route::PublishNamespace,
            Some(_) => Route::Unhandled, // Unroutable RequestOk
            None => Route::NotFound,     // Untracked request
          }
        };

        // 2. Dispatch to the correct handler
        match route {
          Route::TrackStatus => {
            track_status_handler::handle(control_stream_handler, msg, context.clone()).await
          }
          Route::SubscribeNamespace => {
            subscribe_namespace_handler::handle(
              client.clone(),
              control_stream_handler,
              msg,
              context.clone(),
            )
            .await
          }
          Route::PublishNamespace => {
            publish_namespace_handler::handle(
              client.clone(),
              control_stream_handler,
              msg,
              context.clone(),
            )
            .await
          }
          Route::Unhandled => {
            warn!(
              "Router received RequestOk for request_id: {}, but no route is configured for this request type.",
              req_id
            );
            Ok(())
          }
          Route::NotFound => {
            warn!(
              "Router received RequestOk for untracked request_id: {}",
              req_id
            );
            Ok(())
          }
        }
      }
      ControlMessage::Fetch(_) | ControlMessage::FetchCancel(_) | ControlMessage::FetchOk(_) => {
        fetch_handler::handle(client.clone(), control_stream_handler, msg, context.clone()).await
      }
      ControlMessage::Publish(_)
      | ControlMessage::PublishDone(_)
      | ControlMessage::PublishOk(_) => {
        publish_handler::handle(client.clone(), control_stream_handler, msg, context.clone()).await
      }

      ControlMessage::RequestError(err_msg) => {
        let req_id = err_msg.request_id;

        enum Route {
          Fetch,
          Subscribe,
          TrackStatus,
          PublishNamespace,
          SubscribeNamespace,
          Publish,
          NotFound,
        }

        // 1. Peek at the unified map to determine the routing destination
        let route = {
          let map = context.relay_pending_requests.read().await;
          match map.get(&req_id) {
            Some(PendingRequest::Fetch(_)) => Route::Fetch,
            Some(PendingRequest::Subscribe(_)) => Route::Subscribe,
            Some(PendingRequest::TrackStatus(_)) => Route::TrackStatus,
            Some(PendingRequest::PublishNamespace { .. }) => Route::PublishNamespace,
            Some(PendingRequest::SubscribeNamespace { .. }) => Route::SubscribeNamespace,
            Some(PendingRequest::Publish { .. }) => Route::Publish,
            None => Route::NotFound,
          }
        };

        // 2. Dispatch the original message to the correct handler
        match route {
          Route::Fetch => {
            fetch_handler::handle(client.clone(), control_stream_handler, msg, context.clone())
              .await
          }
          Route::Subscribe => {
            subscribe_handler::handle(client.clone(), control_stream_handler, msg, context.clone())
              .await
          }
          Route::TrackStatus => {
            track_status_handler::handle(control_stream_handler, msg, context.clone()).await
          }
          Route::PublishNamespace => {
            publish_namespace_handler::handle(
              client.clone(),
              control_stream_handler,
              msg,
              context.clone(),
            )
            .await
          }
          Route::SubscribeNamespace => {
            subscribe_namespace_handler::handle(
              client.clone(),
              control_stream_handler,
              msg,
              context.clone(),
            )
            .await
          }
          Route::Publish => {
            publish_handler::handle(client.clone(), control_stream_handler, msg, context.clone())
              .await
          }
          Route::NotFound => {
            warn!(
              "Router received RequestError for untracked request_id: {}",
              req_id
            );
            Ok(())
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
