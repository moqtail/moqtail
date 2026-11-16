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

//! Ending a publisher's copy of a track found malformed, whichever path found it: a
//! subgroup stream, a datagram or an upstream FETCH response.

use crate::server::message_handlers::fetch_handler::FetchStop;
use crate::server::message_handlers::subscribe_handler::upstream::remove_upstream_publisher;
use crate::server::session::client::MOQTClient;
use crate::server::session::session_context::SessionContext;
use crate::server::track::{Track, TrackOrigin};
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::data::full_track_name::FullTrackName;
use std::sync::Arc;
use tracing::error;

async fn cancel_downstream_fetches(context: &SessionContext, full_track_name: &FullTrackName) {
  let clients = context
    .client_manager
    .clients
    .read()
    .await
    .values()
    .cloned()
    .collect::<Vec<_>>();
  for client in clients {
    client
      .downstream_requests
      .stop_fetches_of(full_track_name, FetchStop::MalformedTrack)
      .await;
  }
}

/// Ends `publisher`'s copy of a track found malformed. Only the first call for a track
/// tears it down, and returns true: streams, datagrams and upstream FETCHes can find the
/// same track malformed at once, and a second teardown would remove the track by name,
/// even one created since.
pub(crate) async fn handle_malformed_track(
  context: &SessionContext,
  track: &Arc<Track>,
  publisher: &MOQTClient,
) -> bool {
  let (full_track_name, remove_track) = {
    let _handover = track.lock_handover().await;
    if !publisher
      .mark_track_malformed(track.full_track_name.clone())
      .await
    {
      return false;
    }
    cancel_downstream_fetches(context, &track.full_track_name).await;
    track
      .cancel_for_malformed_publisher(publisher.connection_id)
      .await;
    let still_served = remove_upstream_publisher(track, publisher.connection_id, context).await;
    track.remove_from_subscriber_switching_sets().await;
    if let Err(error) = track
      .notify_publish_done(
        PublishDoneStatusCode::MalformedTrack,
        "Malformed track".to_string(),
      )
      .await
    {
      error!("Failed to end subscriptions for malformed track: {error:?}");
    }
    track.subscription_manager.detach_all().await;
    (
      track.full_track_name.clone(),
      track.origin() == TrackOrigin::Subscribe || !still_served,
    )
  };
  if remove_track {
    context.track_manager.remove_track(&full_track_name).await;
  }
  true
}
