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
use moqtail::model::control::subscribe_namespace_ok::SubscribeNamespaceOk;
use moqtail::model::error::TerminationCode;
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

      // 1. Store the Subscription in TrackManager
      // Registers the client's interest with the global router
      context
        .track_manager
        .add_namespace_subscriber(sub_ns.track_namespace_prefix.clone(), client.clone())
        .await;

      // 2. Send OK
      // Note: Assuming SubscribeNamespaceOk::new takes request_id and parameters (or just request_id depending on version)
      // If your library version of SubscribeNamespaceOk::new takes params, use: new(sub_ns.request_id, vec![])
      let ok = SubscribeNamespaceOk::new(sub_ns.request_id);
      handler
        .send(&ControlMessage::SubscribeNamespaceOk(Box::new(ok)))
        .await?;

      info!(
        "Sent SubscribeNamespaceOk for request_id: {}",
        sub_ns.request_id
      );

      // 3. Bootstrap Discovery (Stateful update)
      // Check for EXISTING tracks that match this prefix and notify immediately.
      {
        {
          // OLD: let tracks = context.track_manager.tracks.read().await;
          // NEW: Check Announcements instead of just active tracks
          let matched_announcements = context
            .track_manager
            .get_announcements_by_prefix(&sub_ns.track_namespace_prefix)
            .await;

          for ns in matched_announcements {
            debug!("Found existing announcement for new subscription: {:?}", ns);

            let notify = PublishNamespace::new(sub_ns.request_id, ns, &[]);

            handler
              .send(&ControlMessage::PublishNamespace(Box::new(notify)))
              .await?;
          }
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
