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
use crate::server::session_context::SessionContext;
use core::result::Result;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::{
  control_message::ControlMessage, publish_namespace_ok::PublishNamespaceOk,
};
use moqtail::model::error::TerminationCode;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{info, warn};

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

      // and forward the message to people who are subscribed to the namespace
      {
        let subs_map = context.track_manager.namespace_subscribers.read().await;
        let ns_str = m.track_namespace.to_utf8_path();

        for (prefix, subscribers) in subs_map.iter() {
          if ns_str.starts_with(&prefix.to_utf8_path()) {
            for sub in subscribers {
              // Don't echo back to announcer
              if sub.connection_id == client.connection_id {
                continue;
              }

              info!(
                "Forwarding announcement {:?} to subscriber {}",
                m.track_namespace, sub.connection_id
              );

              let notify = PublishNamespace::new(
                0, // Notification ID
                m.track_namespace.clone(),
                &[],
              );

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
