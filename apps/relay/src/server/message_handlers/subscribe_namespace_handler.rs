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

use crate::server::client::MOQTClient;
use crate::server::session_context::SessionContext;
use core::result::Result;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::control::subscribe_namespace_ok::SubscribeNamespaceOk;
use moqtail::model::error::TerminationCode;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{error, info, warn};

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
        let notify = PublishNamespace::new(0, ns.clone(), &[]);
        if let Err(e) = handler
          .send(&ControlMessage::PublishNamespace(Box::new(notify)))
          .await
        {
          error!("Failed to send catch-up PublishNamespace: {:?}", e);
        }
      }

      // 4. Catch up on active published Tracks
      let matched_tracks = context
        .track_manager
        .get_tracks_and_publishes_by_namespace_prefix(&sub_ns.track_namespace_prefix)
        .await;

      for (_full_track_name, track_arc, original_publish_message) in matched_tracks {
        let notify = original_publish_message.clone();

        if let Err(e) = handler
          .send(&ControlMessage::Publish(Box::new(notify)))
          .await
        {
          error!("Failed to send catch-up Publish msg: {:?}", e);
        }

        // Auto-subscribe the client to the underlying data stream
        let synthetic_sub = Subscribe::new_next_group_start(
          0,
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
          warn!("Failed to auto-subscribe late-joining client: {:?}", e);
        } else {
          info!("Successfully wired data stream for late-joining client.");
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
