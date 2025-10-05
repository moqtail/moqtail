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
use crate::server::track::Track;
use core::result::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
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
      info!("Received Publish message for track: {}", m.track_name);
      let request_id = m.request_id;

      // Check request ID
      {
        let max_request_id = context.max_request_id.read().await;
        if request_id >= *max_request_id {
          warn!(
            "Request ID ({}) is greater than max request ID ({})",
            request_id, max_request_id
          );
          return Err(TerminationCode::TooManyRequests);
        }
      }

      // Validate track namespace authorization
      // TODO: Implement actual authorization logic based on your requirements
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

      // Check if the track already exists and is being published
      // TODO: Actually this should be allowed, multiple publishers can publish the same track
      // but cannot use the same track alias

      {
        if context
          .track_aliases
          .read()
          .await
          .contains_key(&m.track_alias)
        {
          return Err(TerminationCode::DuplicateTrackAlias);
        }
      }

      // Add the track to the client's published tracks
      let full_track_name = moqtail::model::data::full_track_name::FullTrackName {
        namespace: m.track_namespace.clone(),
        name: m.track_name.clone().into(),
      };

      // TODO: what happens multiple publishers publish the same track?
      if !context.tracks.read().await.contains_key(&full_track_name) {
        info!("Track not found, creating new track: {:?}", m.track_alias);
        // subscribed_tracks.insert(sub.track_alias, Track::new(sub.track_alias, track_namespace.clone(), sub.track_name.clone()));
        let track = Track::new(
          m.track_alias,
          m.track_namespace.clone(),
          m.track_name.clone(),
          context.connection_id, // TODO: what happens there are multiple publishers?
          context.server_config,
        );
        {
          context
            .tracks
            .write()
            .await
            .insert(full_track_name.clone(), track.clone());

          // insert the track alias into the track aliases
          context
            .track_aliases
            .write()
            .await
            .insert(m.track_alias, full_track_name.clone());
        }

        client.add_published_track(full_track_name).await;
      } else {
        // a different track alias but same full track name
        // maybe we can associate the track alias with the existing published track
      }

      // Create PublishOk response
      // For simplicity, using default values that can be configured based on requirements
      let publish_ok = Box::new(PublishOk::new(
        request_id,
        m.forward,                // Use the same forward preference as requested
        5,                        // Default subscriber priority
        GroupOrder::Ascending,    // Default group order, could be configurable
        FilterType::LatestObject, // Default filter type
        None,                     // No start location for LatestObject
        None,                     // No end group for LatestObject
        vec![],                   // No additional parameters
      ));

      info!(
        "Accepted publish request for track: {} with alias: {}",
        m.track_name, m.track_alias
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
      // TODO: Implement track cleanup logic based on request_id
      cleanup_published_track(&client, m.request_id).await;

      Ok(())
    }
    _ => {
      // This handler only processes Publish and PublishDone messages
      Ok(())
    }
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

/// Cleans up resources associated with a published track
async fn cleanup_published_track(_client: &Arc<MOQTClient>, _request_id: u64) {
  // TODO: Implement track cleanup logic
  // This could include:
  // - Removing track from active tracks registry
  // - Notifying subscribers about track ending
  // - Cleaning up stream state
  // - Releasing resources

  info!(
    "Cleaning up published track for request ID: {}",
    _request_id
  );
}
