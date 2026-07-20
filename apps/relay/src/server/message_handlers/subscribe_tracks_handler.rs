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
use crate::server::message_handlers::subscribe_namespace_handler::{
  MAX_NAMESPACE_PREFIX_FIELDS, oversized_namespace_error,
};
use crate::server::session::Session;
use crate::server::session_context::{PendingRequest, SessionContext};
use crate::server::track_manager::SubscribeKind;
use core::result::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::request_ok::RequestOk;
use moqtail::model::control::subscribe_tracks::SubscribeTracks;
use moqtail::model::error::{RequestErrorCode, TerminationCode};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

/// SUBSCRIBE_TRACKS: request a PUBLISH for every track already published under
/// the prefix (present and future). Independent overlap space from
/// SUBSCRIBE_NAMESPACE.
pub async fn handle_subscribe_tracks(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  sub_tracks: Box<SubscribeTracks>,
  context: Arc<SessionContext>,
  namespace_tx: UnboundedSender<ControlMessage>,
) -> Result<(), TerminationCode> {
  info!(
    "Received SubscribeTracks message: {:?}",
    sub_tracks.track_namespace_prefix
  );

  if let Some(err) = oversized_namespace_error(&sub_tracks.track_namespace_prefix) {
    warn!(
      "SUBSCRIBE_TRACKS prefix has {} fields, maximum is {}",
      sub_tracks.track_namespace_prefix.fields.len(),
      MAX_NAMESPACE_PREFIX_FIELDS
    );
    stream_handler
      .send(&ControlMessage::RequestError(Box::new(err)))
      .await?;
    return Ok(());
  }

  // Independent overlap space: only other SUBSCRIBE_TRACKS prefixes conflict.
  if let Some(existing_prefix) = context
    .track_manager
    .find_overlapping_namespace_subscription(
      client.connection_id,
      &sub_tracks.track_namespace_prefix,
      SubscribeKind::Tracks,
    )
    .await
  {
    warn!(
      "SUBSCRIBE_TRACKS overlap: new={:?} conflicts with existing={:?}",
      sub_tracks.track_namespace_prefix, existing_prefix
    );
    let err = RequestError::new(
      RequestErrorCode::PrefixOverlap,
      0,
      ReasonPhrase::try_new("Track prefix overlaps with existing subscription".to_string())
        .unwrap(),
    );
    stream_handler
      .send(&ControlMessage::RequestError(Box::new(err)))
      .await?;
    return Ok(());
  }

  context
    .track_manager
    .add_namespace_subscriber(
      sub_tracks.track_namespace_prefix.clone(),
      client.clone(),
      SubscribeKind::Tracks,
      sub_tracks.parameters.clone(),
      namespace_tx,
    )
    .await;

  {
    let mut map = client.inbound_requests.write().await;
    map.insert(
      sub_tracks.request_id,
      PendingRequest::SubscribeTracks {
        client_connection_id: client.connection_id,
        original_request_id: sub_tracks.request_id,
        message: (*sub_tracks).clone(),
      },
    );
  }

  let ok = RequestOk::new(vec![]);
  stream_handler
    .send(&ControlMessage::RequestOk(Box::new(ok)))
    .await?;

  // Forward a PUBLISH for every track already published under the prefix.
  let matched_tracks = context
    .track_manager
    .get_tracks_and_publishes_by_namespace_prefix(&sub_tracks.track_namespace_prefix)
    .await;

  for (full_track_name, track_arc, original_publish_message_opt) in matched_tracks {
    if let Some(mut original_publish_message) = original_publish_message_opt {
      info!("Forwarding existing track to SUBSCRIBE_TRACKS subscriber: {full_track_name:?}");

      let relay_track_id = {
        let track = track_arc.read().await;
        track.relay_track_id
      };

      let relay_publish_id =
        Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;
      original_publish_message.request_id = relay_publish_id;
      original_publish_message.track_alias = relay_track_id;

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

      let track_read = track_arc.read().await;
      if let Err(e) = track_read
        .add_subscription(client.clone(), original_publish_message.clone(), false)
        .await
      {
        warn!("Failed retroactive auto-subscribe for track: {:?}", e);
      }
    } else {
      warn!("The track has no associated publish message, track: {full_track_name:?}");
    }
  }

  Ok(())
}
