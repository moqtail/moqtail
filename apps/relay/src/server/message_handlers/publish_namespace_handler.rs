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
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::namespace::Namespace;
use moqtail::model::control::namespace_done::NamespaceDone;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::{control_message::ControlMessage, request_ok::RequestOk};
use moqtail::model::error::{RequestErrorCode, StreamResetCode, TerminationCode};
use moqtail::model::parameter::message_parameter::apply_message_parameter_update;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{info, warn};

/// Sends one message per matching discovery subscriber, naming the namespace by the
/// suffix that subscriber's own prefix leaves. The announcer is skipped so it is never
/// told about its own namespace.
pub(crate) async fn announce_to_namespace_subscribers(
  context: &Arc<SessionContext>,
  namespace: &Tuple,
  announcer_connection_id: usize,
  message_for: impl Fn(Tuple) -> ControlMessage,
) {
  let subs_map = context.track_manager.namespace_subscribers.read().await;
  for (prefix, subscribers) in subs_map.iter() {
    if !namespace.starts_with(prefix) {
      continue;
    }
    let Some(suffix) = namespace.suffix(prefix) else {
      continue;
    };
    for (sub, kind, _params, namespace_tx) in subscribers {
      if *kind != SubscribeKind::Namespace || sub.connection_id == announcer_connection_id {
        continue;
      }
      info!(
        "Forwarding namespace suffix {:?} to subscriber {}",
        suffix, sub.connection_id
      );
      let _ = namespace_tx.send(message_for(suffix.clone()));
    }
  }
}

/// A namespace is announced for as long as its request stream is open, so closing that
/// stream withdraws it. Everything that named the publisher for this namespace is
/// dropped, and the subscribers that heard the announcement are told it is over.
pub async fn cancel(client: Arc<MOQTClient>, request_id: u64, context: &Arc<SessionContext>) {
  let namespace = {
    let mut map = client.inbound_requests.write().await;
    match map.remove(&request_id) {
      Some(PendingRequest::PublishNamespace { message, .. }) => Some(message.track_namespace),
      _ => None,
    }
  };
  let Some(namespace) = namespace else {
    return;
  };

  client.remove_announced_track_namespace(&namespace).await;

  // A publisher whose announcement was already replaced by another's has nothing to
  // withdraw, and must not send a NAMESPACE_DONE for a namespace that is still served.
  if !context
    .track_manager
    .remove_announcement(&namespace, client.connection_id)
    .await
  {
    return;
  }

  announce_to_namespace_subscribers(context, &namespace, client.connection_id, |s| {
    ControlMessage::NamespaceDone(Box::new(NamespaceDone::new(s)))
  })
  .await;
}

pub async fn handle(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
  opening_request_id: Option<u64>,
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

      // Track in inbound_requests so a REQUEST_UPDATE on this request's stream can find it,
      // and so closing that stream can find the namespace to withdraw.
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

      announce_to_namespace_subscribers(&context, &m.track_namespace, client.connection_id, |s| {
        ControlMessage::Namespace(Box::new(Namespace::new(s)))
      })
      .await;

      let request_ok = Box::new(RequestOk::new(vec![]));

      stream_handler
        .send(&ControlMessage::RequestOk(request_ok))
        .await
    }

    ControlMessage::RequestUpdate(m) => {
      let update_msg = *m;
      let Some(target_request_id) = opening_request_id else {
        return Err(TerminationCode::ProtocolViolation);
      };
      let existing_req_id = target_request_id;

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

      for (session, _) in downstream_sessions {
        if let Some(downstream_req_id) = session.get_outbound_announce_id(&target_namespace).await {
          let relay_update_id = crate::server::session::Session::get_next_relay_request_id(
            context.relay_next_request_id.clone(),
          )
          .await;

          let fanout_msg = moqtail::model::control::request_update::RequestUpdate::new(
            relay_update_id,
            update_msg.parameters.clone(),
          );

          // Goes on the namespace subscription's own request stream, which is
          // what names the request being updated.
          session
            .send_response(
              downstream_req_id,
              ControlMessage::RequestUpdate(Box::new(fanout_msg)),
            )
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
