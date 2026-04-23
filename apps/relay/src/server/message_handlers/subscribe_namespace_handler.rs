// Copyright 2026 The MOQtail Authors
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

use crate::server::session_context::{PendingRequest, SessionContext};
use crate::server::{client::MOQTClient, session::Session};
use core::result::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::RequestErrorCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::request_ok::RequestOk;
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::message_parameter::apply_message_parameter_update;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub async fn handle(
  client: Arc<MOQTClient>,
  handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::SubscribeNamespace(sub_ns) => {
      info!(
        "Received SubscribeNamespace message: {:?}",
        sub_ns.track_namespace_prefix
      );

      // 0. Check for prefix overlap per draft spec (PREFIX_OVERLAP)
      if let Some(existing_prefix) = context
        .track_manager
        .find_overlapping_namespace_subscription(
          client.connection_id,
          &sub_ns.track_namespace_prefix,
        )
        .await
      {
        warn!(
          "SUBSCRIBE_NAMESPACE overlap: new={:?} conflicts with existing={:?}",
          sub_ns.track_namespace_prefix, existing_prefix
        );
        let err = RequestError::new(
          sub_ns.request_id,
          RequestErrorCode::PrefixOverlap,
          0, //TODO: Maybe decide on another retry interval?
          ReasonPhrase::try_new("Namespace prefix overlaps with existing subscription".to_string())
            .unwrap(),
        );
        handler
          .send(&ControlMessage::RequestError(Box::new(err)))
          .await?;
        return Ok(());
      }

      // 1. Store the Subscription in TrackManager
      context
        .track_manager
        .add_namespace_subscriber(
          sub_ns.track_namespace_prefix.clone(),
          client.clone(),
          (*sub_ns).clone(),
        )
        .await;

      {
        let mut map = client.inbound_requests.write().await;
        map.insert(
          sub_ns.request_id,
          PendingRequest::SubscribeNamespace {
            client_connection_id: client.connection_id,
            original_request_id: sub_ns.request_id,
            message: (*sub_ns).clone(),
          },
        );
      }
      // TODO(Draft-16 Stream Split): When SUBSCRIBE_NAMESPACE is moved to its own dedicated
      // bidirectional QUIC stream, we MUST remove this request_id from relay_pending_requests
      // when that underlying QUIC stream receives a FIN or RESET_STREAM.

      // 2. Send OK back to the subscriber
      let ok = RequestOk::new(sub_ns.request_id, vec![]);
      handler
        .send(&ControlMessage::RequestOk(Box::new(ok)))
        .await?;
      info!(
        "Sent RequestOk for SubscribeNamespace request_id: {}",
        sub_ns.request_id
      );

      // 3. Bootstrap Discovery (Catch-up phase for Announcements)
      let matched_announcements = context
        .track_manager
        .get_announcements_by_prefix(&sub_ns.track_namespace_prefix)
        .await;

      for ns in matched_announcements {
        let relay_announce_id =
          Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;
        let notify = PublishNamespace::new(relay_announce_id, ns.clone(), &[]);

        // Register the message in unified map for draft-16 response tracking
        {
          let mut map = context.relay_pending_requests.write().await;
          map.insert(
            relay_announce_id,
            PendingRequest::PublishNamespace {
              client_connection_id: client.connection_id,
              original_request_id: relay_announce_id,
              message: notify.clone(),
            },
          );
        }

        client
          .queue_message(ControlMessage::PublishNamespace(Box::new(notify)))
          .await;
      }

      // 4. Catch up on active published Tracks
      let matched_tracks = context
        .track_manager
        .get_tracks_and_publishes_by_namespace_prefix(&sub_ns.track_namespace_prefix)
        .await;

      for (full_track_name, track_arc, original_publish_message_opt) in matched_tracks {
        if let Some(mut original_publish_message) = original_publish_message_opt {
          info!(
            "Forwarding existing track for new subscriber: {:?}",
            full_track_name
          );

          let relay_track_id = {
            let track = track_arc.read().await;
            track.relay_track_id
          };

          let relay_publish_id =
            Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;
          original_publish_message.request_id = relay_publish_id;
          original_publish_message.track_alias = relay_track_id;

          // Register the message in unified map for draft-16 response tracking
          {
            let mut map = context.relay_pending_requests.write().await;
            map.insert(
              relay_publish_id,
              PendingRequest::Publish {
                publisher_connection_id: client.connection_id,
                original_request_id: relay_publish_id,
                message: original_publish_message.clone(),
              },
            );
          }

          client
            .queue_message(ControlMessage::Publish(Box::new(
              original_publish_message.clone(),
            )))
            .await;

          // Auto-subscribe the client to the underlying data stream
          let track_read = track_arc.read().await;
          if let Err(e) = track_read
            .add_subscription(client.clone(), original_publish_message.clone(), false)
            .await
          {
            warn!("Failed retroactive auto-subscribe for track: {:?}", e);
          } else {
            info!("Successfully initialized retroactive subscription.");
          }
        } else {
          warn!(
            "The track has no associated publish message, track: {:?}",
            full_track_name
          );
        }
      }
    }

    ControlMessage::RequestOk(m) => {
      let msg = *m;

      let mapping = {
        let mut map = context.relay_pending_requests.write().await;
        match map.remove(&msg.request_id) {
          Some(PendingRequest::SubscribeNamespace {
            client_connection_id,
            original_request_id,
            ..
          }) => Some((client_connection_id, original_request_id)),
          Some(_) => {
            warn!(
              "Mismatched request type for RequestOk (SubscribeNamespace): {}",
              msg.request_id
            );
            None
          }
          None => None,
        }
      };

      if let Some((client_connection_id, original_request_id)) = mapping {
        let manager = context.client_manager.read().await;
        if let Some(downstream_client) = manager.get(client_connection_id).await {
          let forwarded_msg = RequestOk::new(original_request_id, msg.parameters);

          info!(
            "Forwarding RequestOk (for SubscribeNamespace) to Client {}",
            client_connection_id
          );

          downstream_client
            .queue_message(ControlMessage::RequestOk(Box::new(forwarded_msg)))
            .await;
        } else {
          warn!("Downstream client {} disconnected", client_connection_id);
        }
      } else {
        debug!(
          "SubscribeNamespace handler received RequestOk for untracked ID: {}",
          msg.request_id
        );
      }
    }

    ControlMessage::RequestUpdate(m) => {
      let update_msg = *m;
      let existing_req_id = update_msg.existing_request_id;
      let update_req_id = update_msg.request_id;

      let target_prefix = {
        let mut map = client.inbound_requests.write().await;
        match map.get_mut(&existing_req_id) {
          Some(PendingRequest::SubscribeNamespace { message, .. }) => {
            apply_message_parameter_update(&mut message.parameters, update_msg.parameters.clone());
            message.track_namespace_prefix.clone()
          }
          _ => {
            warn!(
              "Request {} is not a valid SubscribeNamespace request",
              existing_req_id
            );
            return Err(TerminationCode::ProtocolViolation);
          }
        }
      };

      info!(
        "Processing SUBSCRIBE_NAMESPACE update for prefix: {:?}",
        target_prefix
      );

      context
        .track_manager
        .update_namespace_subscription_parameters(
          &target_prefix,
          client.connection_id,
          update_msg.parameters.clone(),
        )
        .await;

      let ok_msg = RequestOk::new(update_req_id, vec![]);
      handler
        .send(&ControlMessage::RequestOk(Box::new(ok_msg)))
        .await?;
    }

    _ => {
      warn!(
        "Unexpected message in subscribe_namespace_handler: {:?}",
        msg
      );
    }
  }

  Ok(())
}
