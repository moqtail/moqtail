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
use crate::server::session_context::SessionContext;
use crate::server::track::{Track, TrackStatus};
use core::result::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::control::{
  constant::{FilterType, GroupOrder, PublishErrorCode},
  control_message::ControlMessage,
  publish_error::PublishError,
  publish_ok::PublishOk,
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
    ControlMessage::Publish(m) => {
      info!("Received Publish message for track: {:?}", m.track_name);
      let request_id = m.request_id;

      // Check request ID
      {
        let max_request_id = context
          .max_request_id
          .load(std::sync::atomic::Ordering::Relaxed);
        if request_id >= max_request_id {
          warn!(
            "Request ID ({}) is greater than max request ID ({})",
            request_id, max_request_id
          );
          return Err(TerminationCode::TooManyRequests);
        }
      }

      // Validate track namespace authorization
      // TODO: Implement actual authorization logic
      let is_authorized = validate_publish_authorization(&m.track_namespace, &client).await;

      if !is_authorized {
        let reason_phrase =
          ReasonPhrase::try_new("Not authorized to publish this track".to_string())
            .map_err(|_| TerminationCode::InternalError)?;

        let publish_error = Box::new(PublishError::new(
          request_id,
          PublishErrorCode::Unauthorized,
          reason_phrase,
        ));

        return control_stream_handler
          .send(&ControlMessage::PublishError(publish_error))
          .await;
      }

      if context.track_manager.has_track_alias(&m.track_alias).await {
        return Err(TerminationCode::DuplicateTrackAlias);
      }

      // Add the track to the client's published tracks
      let full_track_name = moqtail::model::data::full_track_name::FullTrackName {
        namespace: m.track_namespace.clone(),
        name: m.track_name.clone(),
      };

      let m_clone = m.clone();

      // 1. Get or create the track unconditionally.
      let (track_arc, _is_creator) = context
        .track_manager
        .get_or_create_track(&full_track_name, || {
          Track::new(
            m.track_alias,
            full_track_name.clone(),
            context.connection_id,
            context.server_config,
            TrackStatus::Confirmed {
              publisher_track_alias: m.track_alias,
              expires: 0,
              largest_location: None,
            },
          )
        })
        .await;

      // 2. Unconditionally update the track's alias to the active PUBLISH alias.
      // This heals the state if a SUBSCRIBE previously created a shell track.
      {
        let mut track = track_arc.write().await;
        let status = track.get_status().await;

        // Only update if it's a Pending shell track that never got a SubscribeOk
        if matches!(status, TrackStatus::Pending) {
          track.track_alias = m.track_alias;
          track.confirm(m.track_alias, 0, None).await;
        }
      }

      // 3. Register the new alias so incoming data streams are routed correctly
      context
        .track_manager
        .add_track_alias(m.track_alias, full_track_name.clone())
        .await;

      // 4. Record the publish message so it can be cleanly purged on PublishDone.
      context
        .track_manager
        .add_publish_message(full_track_name.clone(), *m.clone())
        .await;
      client.add_published_track(full_track_name.clone()).await;

      // 5. Forward the new PUBLISH state to all namespace subscribers.
      let subscribers = context
        .track_manager
        .get_namespace_subscribers(&m.track_namespace)
        .await;

      if !subscribers.is_empty() {
        info!(
          "Found {} subscribers for namespace {:?}, forwarding PUBLISH",
          subscribers.len(),
          m.track_namespace
        );
      }

      for subscriber in subscribers {
        info!(
          "Forwarding Publish to interested client: {}",
          subscriber.connection_id
        );

        let relay_req_id =
          Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

        let sub_clone = subscriber.clone();

        context
          .track_manager
          .add_active_push(full_track_name.clone(), subscriber.clone(), relay_req_id)
          .await;

        let mut m_clone2 = m_clone.clone();
        m_clone2.request_id = relay_req_id;

        tokio::spawn(async move {
          sub_clone
            .queue_message(ControlMessage::Publish(m_clone2))
            .await;
        });
      }
      let publish_ok = Box::new(PublishOk::new(
        request_id,
        m_clone.forward,          // Use the same forward preference as requested
        5,                        // Default subscriber priority
        GroupOrder::Ascending,    // Default group order, could be configurable
        FilterType::LatestObject, // Default filter type
        None,                     // No start location for LatestObject
        None,                     // No end group for LatestObject
        vec![],                   // No additional parameters
      ));

      info!(
        "Accepted publish request for track: {:?} with alias: {}",
        m_clone.track_name, m_clone.track_alias
      );

      control_stream_handler
        .send(&ControlMessage::PublishOk(publish_ok))
        .await
    }
    ControlMessage::PublishDone(m) => {
      info!(
        "Received PublishDone message for request ID: {} with status: {:?}",
        m.request_id, m.status_code
      );

      let track_name_opt = context
        .track_manager
        .get_track_name_by_publisher(client.connection_id, m.request_id)
        .await;

      if let Some(track_name) = track_name_opt {
        info!(
          "DEBUG [PublishDone]: Mapped incoming Request ID {} on Conn {} to Track Name: {:?}",
          m.request_id, client.connection_id, track_name
        );

        // Forward PublishDone to all subscribers who accepted the push
        let pushes = context.track_manager.active_pushes.read().await;
        if let Some(subscribers) = pushes.get(&track_name) {
          for (sub_client, sub_req_id) in subscribers {
            info!(
              "DEBUG [PublishDone]: --> Forwarding PublishDone to client {} with mapped Request ID {}",
              sub_client.connection_id, sub_req_id
            );
            let mut done_msg = (*m).clone();
            done_msg.request_id = *sub_req_id; // Remap it to the ID the peer expects

            let sub_clone = sub_client.clone();
            tokio::spawn(async move {
              sub_clone
                .queue_message(ControlMessage::PublishDone(Box::new(done_msg)))
                .await;
            });
          }
        }
        drop(pushes);

        // Purge the track completely
        let track_alias = {
          let publishes = context.track_manager.publishes.read().await;
          publishes.get(&track_name).map(|p| p.track_alias)
        };

        if let Some(alias) = track_alias {
          context.track_manager.remove_track_by_alias(alias).await;
        } else {
          context.track_manager.remove_track(&track_name).await;
        }
      } else {
        warn!(
          "DEBUG [PublishDone]: FAILED to find a track matching Request ID {} for Connection {}!",
          m.request_id, client.connection_id
        );
      }

      Ok(())
    }

    ControlMessage::PublishOk(m) => {
      info!("Received PublishOk for request_id: {}", m.request_id);

      // 1. Look up which track this PublishOk corresponds to using the connection_id and request_id
      if let Some(full_track_name) = context
        .track_manager
        .get_track_name_by_push_id(client.connection_id, m.request_id)
        .await
      {
        // 2. Fetch the track and the original publish message details
        if let Some(track_arc) = context.track_manager.get_track(&full_track_name).await
          && let Some(orig_publish) = context
            .track_manager
            .get_publish_message(&full_track_name)
            .await
        {
          info!(
            "Publish accepted! Wiring up data stream for: {:?}",
            full_track_name
          );

          // 3. Create the subscription now that the client has consented
          let synthetic_sub = Subscribe::new_next_group_start(
            0,
            orig_publish.track_namespace.clone(),
            orig_publish.track_name.clone(),
            128,
            orig_publish.group_order,
            orig_publish.forward != 0,
            vec![],
          );

          let track_read = track_arc.read().await;
          if let Err(e) = track_read
            .add_subscription(client.clone(), synthetic_sub, false)
            .await
          {
            warn!("Failed to auto-subscribe client after PublishOk: {:?}", e);
          } else {
            info!("Successfully wired data stream!");
          }
        }
      } else {
        warn!(
          "Received PublishOk for an unknown push request_id: {}",
          m.request_id
        );
      }

      Ok(())
    }
    _ => Ok(()),
  }
}

/// Validates if the client is authorized to publish to the given track namespace
async fn validate_publish_authorization(
  _track_namespace: &moqtail::model::common::tuple::Tuple,
  _client: &Arc<MOQTClient>,
) -> bool {
  // TODO: Implement actual authorization logic
  // This could check:
  // - Client authentication credentials
  // - Track namespace permissions
  // - Rate limiting
  // - Subscription quotas

  // For now, allow all publishes (this should be replaced with actual auth logic)
  true
}

/// Checks if a track with the given namespace and name already exists
async fn _check_track_exists(
  _track_namespace: &moqtail::model::common::tuple::Tuple,
  _track_name: &str,
) -> bool {
  // TODO: Implement actual track existence checking
  // This could check:
  // - Active tracks registry
  // - Database of existing tracks
  // - Track metadata storage

  // For now, assume no conflicts (this should be replaced with actual logic)
  false
}
