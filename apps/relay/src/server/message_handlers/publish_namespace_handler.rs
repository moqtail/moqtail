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
use moqtail::model::control::{control_message::ControlMessage, request_ok::RequestOk};
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::message_parameter::apply_message_parameter_update;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{debug, info, warn};

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
      client
        .add_announced_track_namespace(m.track_namespace.clone())
        .await;

      // save the namespace among announcements
      // so we can forward it to clients who later come in with subscribe_namespace
      context
        .track_manager
        .add_announcement(m.track_namespace.clone(), client.clone(), (*m).clone())
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
            for (sub, _subscribe_ns_message) in subscribers {
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
              sub
                .register_outbound_announce_id(m.track_namespace.clone(), relay_announce_id)
                .await;

              let notify = PublishNamespace::new(relay_announce_id, m.track_namespace.clone(), &[]);

              // Register the message in unified map for draft-16 response tracking
              {
                let mut map = client.inbound_requests.write().await;
                map.insert(
                  m.request_id,
                  PendingRequest::PublishNamespace {
                    client_connection_id: client.connection_id,
                    original_request_id: m.request_id,
                    message: (*m).clone(),
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

      let request_ok = Box::new(RequestOk::new(
        m.request_id,
        vec![], // No parameters needed for a basic namespace OK
      ));

      control_stream_handler
        .send(&ControlMessage::RequestOk(request_ok))
        .await
    }

    ControlMessage::RequestOk(m) => {
      let msg = *m;

      let mapping = {
        let mut map = context.relay_pending_requests.write().await;
        match map.remove(&msg.request_id) {
          Some(PendingRequest::PublishNamespace {
            client_connection_id,
            ..
          }) => Some(client_connection_id),
          Some(_) => {
            warn!(
              "Mismatched request type for RequestOk (PublishNamespace): {}",
              msg.request_id
            );
            None
          }
          None => None,
        }
      };

      if let Some(client_connection_id) = mapping {
        debug!(
          "Received acknowledgment (RequestOk) from subscriber {} for PublishNamespace broadcast",
          client_connection_id
        );
      } else {
        debug!(
          "PublishNamespace handler received RequestOk for untracked ID: {}",
          msg.request_id
        );
      }
      Ok(())
    }

    ControlMessage::RequestUpdate(m) => {
      let update_msg = *m;
      let existing_req_id = update_msg.existing_request_id;
      let update_req_id = update_msg.request_id;

      let target_namespace = {
        let mut map = client.inbound_requests.write().await;
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

      context
        .track_manager
        .update_namespace_parameters(&target_namespace, update_msg.parameters.clone())
        .await;

      let downstream_sessions = context
        .track_manager
        .get_namespace_subscribers(&target_namespace)
        .await;

      for session in downstream_sessions {
        if let Some(downstream_req_id) = session.get_outbound_announce_id(&target_namespace).await {
          let relay_update_id = crate::server::session::Session::get_next_relay_request_id(
            context.relay_next_request_id.clone(),
          )
          .await;

          let fanout_msg = moqtail::model::control::request_update::RequestUpdate::new(
            relay_update_id,
            downstream_req_id,
            update_msg.parameters.clone(),
          );

          {
            let mut map = context.relay_pending_requests.write().await;
            map.insert(
              relay_update_id,
              PendingRequest::RequestUpdate {
                client_connection_id: session.connection_id,
                original_request_id: relay_update_id,
                message: fanout_msg.clone(),
              },
            );
          }

          session
            .queue_message(ControlMessage::RequestUpdate(Box::new(fanout_msg)))
            .await;
        } else {
          warn!(
            "Found downstream session {} for namespace {:?} but no outbound announce ID was tracked.",
            session.connection_id, target_namespace
          );
        }
      }

      let ok_msg = RequestOk::new(update_req_id, vec![]);
      control_stream_handler
        .send(&ControlMessage::RequestOk(Box::new(ok_msg)))
        .await?;

      Ok(())
    }
    _ => {
      // no-op
      Ok(())
    }
  }
}
