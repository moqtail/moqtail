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
use crate::server::session_context::PendingRequest;
use crate::server::session_context::SessionContext;
use crate::server::track::{Track, TrackStatus};
use core::result::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::{
  constant::{FilterType, GroupOrder, PublishErrorCode},
  control_message::ControlMessage,
  publish_error::PublishError,
  publish_ok::PublishOk,
};
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::constant::MessageParameterType;
use moqtail::model::parameter::message_parameter::apply_message_parameter_update;
use moqtail::model::parameter::message_parameter::{MessageParameter, MessageParameterVecExt};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

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
      let track_alias = m.track_alias;

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

      // Build the full track name early so we can check it against any existing alias mapping.
      let full_track_name = moqtail::model::data::full_track_name::FullTrackName {
        namespace: m.track_namespace.clone(),
        name: m.track_name.clone(),
      };

      // Multiple publishers may share the same alias for the same track (fan-out).
      // Only reject if the alias already maps to a different full track name for this connection.
      {
        if context
          .track_manager
          .has_track_alias(context.connection_id, &m.track_alias)
          .await
        {
          let is_same_track = if let Some(existing) = context
            .track_manager
            .get_track_by_alias(context.connection_id, m.track_alias)
            .await
          {
            existing.read().await.full_track_name == full_track_name
          } else {
            false
          };
          if !is_same_track {
            return Err(TerminationCode::DuplicateTrackAlias);
          }
          // Same track, same alias — fall through to the has_track branch below.
        }
      }

      if !context.track_manager.has_track(&full_track_name).await {
        info!(
          "Track not found, creating new track for publisher alias={}",
          m.track_alias
        );
        let relay_track_id = context.track_manager.generate_relay_track_id();
        let track = Track::new(
          relay_track_id,
          full_track_name.clone(),
          context.server_config,
          TrackStatus::Confirmed {
            subscribe_parameters: vec![],
          },
        );
        let track_arc = context
          .track_manager
          .add_track(
            context.connection_id,
            m.track_alias,
            full_track_name.clone(),
            track,
          )
          .await;
        {
          let track = track_arc.write().await;
          track
            .add_publisher(context.connection_id, track_alias)
            .await;
          track.set_track_extensions(m.track_extensions.clone()).await;
        }

        client
          .add_published_track(request_id, full_track_name.clone())
          .await;

        // register this publish message
        context
          .track_manager
          .add_publish_message(full_track_name.clone(), context.connection_id, (*m).clone())
          .await;

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

          let sub_clone = subscriber.clone();

          let mut m_clone = m.clone();

          let relay_req_id =
            Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

          m_clone.request_id = relay_req_id;
          m_clone.track_alias = relay_track_id;

          // Register the message in unified map for draft-16 response tracking
          {
            let mut map = context.relay_pending_requests.write().await;
            map.insert(
              relay_req_id,
              PendingRequest::Publish {
                publisher_connection_id: context.connection_id,
                original_request_id: request_id,
                message: (*m_clone).clone(),
              },
            );
          }

          let m_clone2 = m_clone.clone();
          tokio::spawn(async move {
            sub_clone
              .queue_message(ControlMessage::Publish(m_clone2))
              .await;
          });

          let track_write = track_arc.read().await;
          if let Err(e) = track_write
            .add_subscription(subscriber.clone(), (*m_clone).clone(), false)
            .await
          {
            warn!(
              "Failed to auto-subscribe client {} to pushed track: {:?}",
              subscriber.connection_id, e
            );
          }
        }
      } else {
        // Another publisher for the same track with a different alias.
        // Register their alias so their data stream can be routed to the existing track.
        context
          .track_manager
          .add_track_alias(
            context.connection_id,
            m.track_alias,
            full_track_name.clone(),
          )
          .await;
        if let Some(track_arc) = context.track_manager.get_track(&full_track_name).await {
          track_arc
            .write()
            .await
            .add_publisher(context.connection_id, m.track_alias)
            .await;
        }
        client
          .add_published_track(request_id, full_track_name.clone())
          .await;
        info!(
          "Additional publisher for existing track {:?}/{}: registered alias {}",
          m.track_namespace, m.track_name, m.track_alias
        );
      }

      let m_clone = m.clone();
      let publish_forward_param = m_clone.parameters.get_param_or(
        MessageParameterType::Forward,
        MessageParameter::new_forward(true),
      );
      let publish_ok = Box::new(PublishOk::new(
        request_id,
        vec![
          publish_forward_param,
          MessageParameter::new_subscriber_priority(5),
          MessageParameter::new_group_order(GroupOrder::Ascending),
          MessageParameter::new_subscription_filter(FilterType::LatestObject, None, None),
        ],
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

      // Clean up the published track
      cleanup_published_track(&client, m.request_id, &context).await;

      // Remove the request from the unified map to avoid memory leak
      {
        let mut map = context.relay_pending_requests.write().await;
        map.remove(&m.request_id);
        debug!(
          "Removed terminated PUBLISH request {} from pending requests map",
          m.request_id
        );
      }

      Ok(())
    }

    ControlMessage::PublishOk(m) => {
      info!("Received PublishOk for request_id: {}", m.request_id);

      let pending_request = {
        let map = context.relay_pending_requests.read().await;
        map.get(&m.request_id).cloned()
      };

      match pending_request {
        Some(PendingRequest::Publish { .. }) => {
          // 1. Look up which track this PublishOk corresponds to using the connection_id and request_id
          if let Some(full_track_name) = context
            .track_manager
            .get_track_name_by_publisher(client.connection_id, m.request_id)
            .await
          {
            // 2. Fetch the track and the original publish message details
            if let Some(track_arc) = context.track_manager.get_track(&full_track_name).await
              && let Some(orig_publish) = context
                .track_manager
                .get_publish_message(&full_track_name, client.connection_id)
                .await
            {
              info!(
                "Publish accepted! Wiring up data stream for: {:?}",
                full_track_name
              );

              let track_read = track_arc.read().await;
              if let Err(e) = track_read
                .add_subscription(client.clone(), orig_publish.clone(), false)
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
        }
        Some(_) => {
          warn!("Mismatched request type for PublishOk: {}", m.request_id);
        }
        None => {
          warn!(
            "Received PublishOk for untracked request_id: {}",
            m.request_id
          );
        }
      }

      Ok(())
    }
    _ => Ok(()),
  }
}

pub async fn handle_request_update(
  client: Arc<MOQTClient>,
  _control_stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  let update_msg = match msg {
    ControlMessage::RequestUpdate(m) => *m,
    _ => {
      error!("publish_handler::handle_request_update called with wrong message type");
      return Err(TerminationCode::InternalError);
    }
  };

  let publisher_req_id = update_msg.existing_request_id;
  // let new_req_id = update_msg.request_id; // TODO: Uncomment when sending RequestOk

  {
    let mut map = context.relay_pending_requests.write().await;
    match map.get_mut(&publisher_req_id) {
      Some(PendingRequest::Publish { message, .. }) => {
        apply_message_parameter_update(&mut message.parameters, update_msg.parameters.clone());
      }
      _ => {
        warn!(
          "Request {} is not a valid Publish request",
          publisher_req_id
        );
        return Err(TerminationCode::ProtocolViolation);
      }
    }
  }

  // 2. Look up the track this publisher owns
  let full_track_name = match context
    .track_manager
    .get_track_name_by_publisher(client.connection_id, publisher_req_id)
    .await
  {
    Some(name) => name,
    None => {
      warn!(
        "No active track found for publisher request {}",
        publisher_req_id
      );
      return Err(TerminationCode::ProtocolViolation);
    }
  };

  let track_arc = match context.track_manager.get_track(&full_track_name).await {
    Some(t) => t,
    None => {
      warn!("Track metadata missing for {:?}", full_track_name);
      return Err(TerminationCode::InternalError);
    }
  };

  info!(
    "Processing Publish REQUEST_UPDATE for track {:?}",
    full_track_name
  );

  // 3. Update the Track's global metadata
  context
    .track_manager
    .update_publish_message_parameters(
      &full_track_name,
      client.connection_id,
      &update_msg.parameters,
    )
    .await;

  // 4. FAN-OUT: Translate the IDs and notify all downstream subscribers
  let active_subscriptions = {
    let track_read = track_arc.read().await;
    track_read
      .subscription_manager
      .get_all_subscriptions()
      .await
  };

  if active_subscriptions.is_empty() {
    info!(
      "No active subscribers for track {:?}, skipping fan-out.",
      full_track_name
    );
  } else {
    info!(
      "Fanning out Publish update to {} subscribers",
      active_subscriptions.len()
    );
  }

  for sub_lock in active_subscriptions {
    let sub = sub_lock.read().await;
    let subscriber_client = sub.subscriber().clone();

    let subscriber_existing_id = sub.request_id;

    let relay_update_id =
      Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

    let mut forwarded_update = update_msg.clone();
    forwarded_update.request_id = relay_update_id;
    forwarded_update.existing_request_id = subscriber_existing_id;

    subscriber_client
      .queue_message(ControlMessage::RequestUpdate(Box::new(forwarded_update)))
      .await;
  }

  // 5. Send RequestOk back to the Publisher acknowledging the update was processed
  // TODO: Uncomment after merging RequestOk/Error support
  /*
  let ok_msg = RequestOk::new(new_req_id);
  _control_stream_handler.send_message(Box::new(ControlMessage::RequestOk(Box::new(ok_msg)))).await?;
  */

  Ok(())
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

/// Cleans up resources associated with a published track.
/// Removes the publisher from its track; if it was the last publisher,
/// remove_publisher() internally notifies subscribers.
async fn cleanup_published_track(
  client: &Arc<MOQTClient>,
  request_id: u64,
  context: &Arc<SessionContext>,
) {
  let full_track_name = {
    let published_tracks = client.published_tracks.read().await;
    published_tracks.get(&request_id).cloned()
  };

  let full_track_name = match full_track_name {
    Some(n) => n,
    None => {
      info!(
        "cleanup_published_track: no track found for request_id={}",
        request_id
      );
      return;
    }
  };

  let track_arc = match context.track_manager.get_track(&full_track_name).await {
    Some(t) => t,
    None => {
      info!(
        "cleanup_published_track: track not in manager for request_id={}",
        request_id
      );
      return;
    }
  };

  let track = track_arc.read().await;
  if let Some(alias) = track.remove_publisher(client.connection_id).await {
    context
      .track_manager
      .remove_publisher_alias(client.connection_id, alias)
      .await;
    if !track.has_publishers().await {
      drop(track);
      context.track_manager.remove_track(&full_track_name).await;
      info!(
        "cleanup_published_track: removed track {:?} (no publishers left) request_id={}",
        full_track_name, request_id
      );
    }
  }
}
