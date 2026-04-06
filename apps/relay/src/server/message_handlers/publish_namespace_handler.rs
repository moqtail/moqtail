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
use crate::server::session::Session;
use crate::server::session_context::{PendingRequest, SessionContext};
use core::result::Result;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::{
  control_message::ControlMessage, publish_namespace_ok::PublishNamespaceOk,
};
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::message_parameter::apply_message_parameter_update;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{error, info, warn};

pub async fn handle(
  client: Arc<MOQTClient>,
  control_stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::PublishNamespace(m) => {
      // TODO: the namespace is already announced, return error
      info!("received PublishNamespace message");
      let request_id = m.request_id;

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

      // this is a publisher, add it to the client manager
      // send announce_ok
      client
        .add_announced_track_namespace(m.track_namespace.clone())
        .await;

      // save the namespace among announcements
      // so we can forward it to clients who later come in with subscribe_namespace
      context
        .track_manager
        .add_announcement(m.track_namespace.clone(), client.clone())
        .await;

      {
        let mut map = context.relay_pending_requests.write().await;
        map.insert(
          m.request_id,
          PendingRequest::PublishNamespace {
            client_connection_id: client.connection_id,
            original_request_id: m.request_id,
            message: (*m).clone(),
          },
        );
      }
      // TODO: Remove this request_id from relay_pending_requests when a PUBLISH_NAMESPACE_DONE
      // is received for this namespace, or if the client disconnects. Until then this is a memory leak.

      // and forward the message to people who are subscribed to the namespace
      {
        let subs_map = context.track_manager.namespace_subscribers.read().await;
        for (prefix, subscribers) in subs_map.iter() {
          if m.track_namespace.starts_with(prefix) {
            for sub in subscribers {
              // Don't echo back to announcer
              if sub.connection_id == client.connection_id {
                continue;
              }

              info!(
                "Forwarding announcement {:?} to subscriber {}",
                m.track_namespace, sub.connection_id
              );
              let relay_announce_id =
                Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

              let notify = PublishNamespace::new(relay_announce_id, m.track_namespace.clone(), &[]);

              // Register the message in unified map for draft-16 response tracking
              {
                let mut map = context.relay_pending_requests.write().await;
                map.insert(
                  relay_announce_id,
                  PendingRequest::PublishNamespace {
                    client_connection_id: sub.connection_id,
                    original_request_id: relay_announce_id, // We are the origin here
                    message: notify.clone(),
                  },
                );
              }

              let sub_clone = sub.clone();
              tokio::spawn(async move {
                let _ = sub_clone
                  .queue_message(ControlMessage::PublishNamespace(Box::new(notify)))
                  .await;
              });
            }
          }
        }
      }

      let announce_ok = Box::new(PublishNamespaceOk {
        request_id: m.request_id,
      });
      control_stream_handler
        .send(&ControlMessage::PublishNamespaceOk(announce_ok))
        .await
    }
    _ => {
      // no-op
      Ok(())
    }
  }
}
pub async fn handle_request_update(
  _client: Arc<MOQTClient>,
  _handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  let update_msg = match msg {
    ControlMessage::RequestUpdate(m) => *m,
    _ => {
      error!("publish_namespace_handler::handle_request_update called with wrong message type");
      return Err(TerminationCode::InternalError);
    }
  };

  let existing_req_id = update_msg.existing_request_id;
  // let new_req_id = update_msg.request_id; // TODO: Uncomment when sending RequestOk

  // 1. Verify this is a PublishNamespace request and apply parameter updates
  let target_namespace = {
    let mut map = context.relay_pending_requests.write().await;
    match map.get_mut(&existing_req_id) {
      Some(PendingRequest::PublishNamespace { message, .. }) => {
        apply_message_parameter_update(&mut message.parameters, update_msg.parameters.clone());
        message.track_namespace.clone()
      }
      _ => {
        warn!(
          "Request {} is not a valid PublishNamespace request",
          existing_req_id
        );
        return Err(TerminationCode::ProtocolViolation);
      }
    }
  };

  info!(
    "Processing PUBLISH_NAMESPACE update for namespace: {:?}",
    target_namespace
  );

  // TODO: Requires bi-directional Request ID mapping to fan-out
  // 2. Update Global Track Manager State
  /*
  context
    .track_manager
    .update_namespace_parameters(&target_namespace, _client.connection_id, update_msg.parameters.clone())
    .await;
  */

  // 3. The Ripple Effect Fan-Out
  /*
  let downstream_sessions = context
    .track_manager
    .get_namespace_subscribers(&target_namespace)
    .await;

  for session in downstream_sessions {
      if let Some(downstream_req_id) = session.get_outbound_announce_id(&target_namespace) {
          let fanout_msg = crate::model::control::request_update::RequestUpdate::new(
              session.generate_request_id(),
              downstream_req_id,
              update_msg.parameters.clone(),
          );

          session.send_control_message(ControlMessage::RequestUpdate(Box::new(fanout_msg))).await?;
      }
  }
  */

  // 4. Acknowledge the update to the Publisher
  /*
  let ok_msg = RequestOk::new(new_req_id);
  _handler.send(&ControlMessage::RequestOk(Box::new(ok_msg))).await?;
  */

  Ok(())
}
