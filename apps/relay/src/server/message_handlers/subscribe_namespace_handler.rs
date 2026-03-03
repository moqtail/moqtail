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

use crate::server::session_context::SessionContext;
use crate::server::{client::MOQTClient, session::Session};
use core::result::Result;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::control::subscribe_namespace_ok::SubscribeNamespaceOk;
use moqtail::model::error::TerminationCode;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{info, warn};

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

      // 1. Store the Subscription in TrackManager
      context
        .track_manager
        .add_namespace_subscriber(sub_ns.track_namespace_prefix.clone(), client.clone())
        .await;

      // 2. Send OK back to the subscriber
      let ok = SubscribeNamespaceOk::new(sub_ns.request_id);
      handler
        .send(&ControlMessage::SubscribeNamespaceOk(Box::new(ok)))
        .await?;
      info!(
        "Sent SubscribeNamespaceOk for request_id: {}",
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
        client
          .queue_message(ControlMessage::PublishNamespace(Box::new(notify)))
          .await;
      }

      // 4. Catch up on active published Tracks
      let matched_tracks = context
        .track_manager
        .get_tracks_and_publishes_by_namespace_prefix(&sub_ns.track_namespace_prefix)
        .await;

      for (full_track_name, track_arc, mut original_publish_message) in matched_tracks {
        info!(
          "Forwarding existing track for new subscriber: {:?}",
          full_track_name
        );

        let relay_publish_id =
          Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;
        original_publish_message.request_id = relay_publish_id;

        // Record the PUSH so PublishDone can reach this subscriber even though they joined after the track started.
        context
          .track_manager
          .add_active_push(full_track_name.clone(), client.clone(), relay_publish_id)
          .await;

        client
          .queue_message(ControlMessage::Publish(Box::new(
            original_publish_message.clone(),
          )))
          .await;

        // Auto-subscribe the client to the underlying data stream
        let synthetic_sub = Subscribe::new_next_group_start(
          relay_publish_id,
          original_publish_message.track_namespace.clone(),
          original_publish_message.track_name.clone(),
          128,
          original_publish_message.group_order,
          original_publish_message.forward != 0,
          vec![],
        );

        let track_read = track_arc.read().await;
        if let Err(e) = track_read
          .add_subscription(client.clone(), synthetic_sub, false)
          .await
        {
          warn!("Failed retroactive auto-subscribe for track: {:?}", e);
        } else {
          info!("Successfully initialized retroactive subscription.");
        }
      }
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
