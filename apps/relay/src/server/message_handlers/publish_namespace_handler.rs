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
use crate::server::session_context::{PendingRequest, SessionContext};
use crate::server::track_manager::SubscribeKind;
use core::result::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::namespace::Namespace;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::{control_message::ControlMessage, request_ok::RequestOk};
use moqtail::model::error::{RequestErrorCode, StreamResetCode, TerminationCode};
use moqtail::model::parameter::message_parameter::apply_message_parameter_update;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{info, warn};

pub async fn handle(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::PublishNamespace(m) => {
      // TODO: the namespace is already announced, return error
      info!("received PublishNamespace message");

      // Namespaces MUST NOT be published under a reserved namespace.
      if m.track_namespace.is_reserved_dot() || m.track_namespace.is_session_namespace() {
        info!("Rejecting PUBLISH_NAMESPACE for reserved namespace");
        let err = RequestError::new(
          RequestErrorCode::DoesNotExist,
          0,
          ReasonPhrase::try_new("reserved namespace cannot be published".to_string()).unwrap(),
        );
        stream_handler
          .send(&ControlMessage::RequestError(Box::new(err)))
          .await?;
        return Ok(());
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

      // Forward the announcement to namespace subscribers via their bi-stream channels
      {
        let subs_map = context.track_manager.namespace_subscribers.read().await;
        for (prefix, subscribers) in subs_map.iter() {
          if m.track_namespace.starts_with(prefix) {
            for (sub, kind, _params, namespace_tx) in subscribers {
              // NAMESPACE advertisements go to discovery (SUBSCRIBE_NAMESPACE) subscribers.
              if *kind != SubscribeKind::Namespace {
                continue;
              }
              // Don't echo back to announcer
              if sub.connection_id == client.connection_id {
                continue;
              }

              if let Some(suffix) = m.track_namespace.suffix(prefix) {
                info!(
                  "Forwarding NAMESPACE suffix {:?} to subscriber {}",
                  suffix, sub.connection_id
                );
                let ns_msg = ControlMessage::Namespace(Box::new(Namespace::new(suffix)));
                let _ = namespace_tx.send(ns_msg);
              }
            }
          }
        }
      }

      let request_ok = Box::new(RequestOk::new(vec![]));

      stream_handler
        .send(&ControlMessage::RequestOk(request_ok))
        .await
    }

    ControlMessage::RequestUpdate(m) => {
      let update_msg = *m;
      let existing_req_id = update_msg.existing_request_id;

      let target_namespace = {
        let mut map = client.inbound_requests.write().await;
        match map.get_mut(&existing_req_id) {
          Some(PendingRequest::PublishNamespace { message, .. }) => {
            apply_message_parameter_update(&mut message.parameters, update_msg.parameters.clone());
            message.track_namespace.clone()
          }
          _ => {
            warn!(
              "REQUEST_UPDATE for PUBLISH_NAMESPACE request {} cannot be applied; closing the stream",
              existing_req_id
            );
            stream_handler.reset(StreamResetCode::Cancelled.to_u64());
            return Ok(());
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
        .get_namespace_subscribers(&target_namespace, SubscribeKind::Namespace)
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

      let ok_msg = RequestOk::new(vec![]);
      stream_handler
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
