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

//! The upstream half of FETCH: asking the publisher for what the cache lacks, and reading its answer.

use super::{FetchStop, UPSTREAM_FETCH_CHANNEL_CAPACITY, UpstreamFetch, UpstreamFetchResponse};
use crate::server::cluster;
use crate::server::message_handlers::parameters;
use crate::server::session::client::MOQTClient;
use crate::server::session::session_context::{SessionContext, UpstreamFetchEvent};
use crate::server::track::{Track, TrackOrigin, UpstreamRequestCancellation, UpstreamRequestKind};
use core::result::Result::{Err, Ok};
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::Fetch;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::error::StreamResetCode;
use moqtail::model::parameter::location_filter::LocationFilter;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::FetchRequest;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, trace, warn};

/// Send an upstream Fetch to the publisher for the range `filter` selects, asking for
/// the groups in `group_order` so they arrive in the order they are written downstream.
/// Returns the upstream request state, including separate channels for the control
/// response and Object events.
pub(crate) async fn send_upstream_fetch_for_range(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  track: &Arc<crate::server::track::Track>,
  filter: LocationFilter,
  group_order: GroupOrder,
) -> Option<UpstreamFetch> {
  let (full_track_name, publisher_aliases) = {
    (
      track.full_track_name.clone(),
      track.publisher_aliases.clone(),
    )
  };
  // A FETCH goes to one publisher, unlike a SUBSCRIBE: any of the matching ones may
  // serve the range, so the first will do.
  let publisher = {
    let publishers = context
      .client_manager
      .get_publishers_for_track(&full_track_name)
      .await;
    cluster::select(context, client, &full_track_name, publishers)
      .await
      .into_iter()
      .next()
  };
  let publisher = match publisher {
    Some(p) => p,
    None => {
      info!(
        "send_upstream_fetch_for_range | No publisher found for {:?}",
        &full_track_name
      );
      return None;
    }
  };

  let relay_request_id = publisher.next_request_id(&context.relay_next_request_id);

  // Delivery treats anything but Descending as Ascending, so upstream is asked the same.
  let group_order = match group_order {
    GroupOrder::Descending => GroupOrder::Descending,
    GroupOrder::Ascending | GroupOrder::Original => GroupOrder::Ascending,
  };
  let upstream_fetch = Fetch::new_with_location_filter(
    relay_request_id,
    full_track_name.namespace.clone(),
    full_track_name.name.clone(),
    filter,
    parameters::upstream_fetch(group_order),
  );

  let (upstream_tx, upstream_rx) = mpsc::channel(UPSTREAM_FETCH_CHANNEL_CAPACITY);
  let (response_tx, response_rx) = oneshot::channel();

  // FETCH is Request, First: it opens its own bidirectional stream. Open it before
  // registering the request so a failure here leaves no state behind.
  let (send, recv) = match publisher.connection.open_bi().await {
    Ok(streams) => streams,
    Err(e) => {
      warn!(
        "send_upstream_fetch_for_range | Failed to open upstream fetch stream: {:?}",
        e
      );
      return None;
    }
  };
  let mut upstream = ControlStreamHandler::new(send, recv).with_peer_id(publisher.connection_id);

  {
    // The response stream finds its track by the Full Track Name of this request, so a
    // publisher that has given the relay no alias for the track can still serve it.
    let publisher_alias = publisher_aliases
      .read()
      .await
      .get(&publisher.connection_id)
      .copied()
      .unwrap_or_default();
    let req = FetchRequest {
      original_request_id: relay_request_id,
      requested_by: client.connection_id,
      fetch_request: upstream_fetch.clone(),
      track_alias: publisher_alias,
    };
    publisher
      .outgoing_fetch_requests
      .write()
      .await
      .insert(relay_request_id, req);
    context
      .upstream_requests
      .add_fetch(relay_request_id, upstream_tx.clone())
      .await;
  }

  // Sent only once the sender is registered, so Objects arriving first are not dropped.
  let mut cancel_rx = track
    .register_upstream_request(
      relay_request_id,
      publisher.connection_id,
      UpstreamRequestKind::Fetch,
    )
    .await;
  if publisher.is_track_malformed(&full_track_name).await {
    track
      .cancel_upstream_request(
        relay_request_id,
        UpstreamRequestCancellation::MalformedTrack,
      )
      .await;
  }
  if let Err(e) = upstream
    .send(&ControlMessage::Fetch(Box::new(upstream_fetch)))
    .await
  {
    warn!(
      "send_upstream_fetch_for_range | Failed to send upstream FETCH: {:?}",
      e
    );
    track.unregister_upstream_request(relay_request_id).await;
    forget_upstream_fetch(context, &publisher, relay_request_id).await;
    return None;
  }

  // The response can come at any time relative to object delivery, so it is read on a
  // task. Holding the handler keeps the request stream open for the fetch's lifetime.
  // Whoever reads the Objects is done with the fetch once it drops its receiver, on
  // whatever path: the task then closes the request stream if it is still open, with
  // a FIN when the whole response arrived and a cancel otherwise, and forgets the fetch.
  let response_event_tx = upstream_tx;
  let upstream_publisher = publisher.clone();
  let context = context.clone();
  let request_track = track.clone();
  let closed_cleanly = Arc::new(AtomicBool::new(false));
  tokio::spawn({
    let closed_cleanly = closed_cleanly.clone();
    async move {
      let mut response_tx = Some(response_tx);
      let response = async {
        match upstream.next_message().await {
          Ok(ControlMessage::FetchOk(ok)) => {
            info!(
              "Upstream FETCH {} accepted, end location {:?}",
              relay_request_id, ok.end_location
            );
            let end_location = ok.end_location.clone();
            // Signal range resolution before writing to the potentially full event queue.
            let _ = response_tx
              .take()
              .unwrap()
              .send(UpstreamFetchResponse::Accepted {
                end_location: end_location.clone(),
                end_of_track: ok.end_of_track,
              });
            let _ = response_event_tx
              .send(UpstreamFetchEvent::Accepted { end_location })
              .await;
          }
          Ok(ControlMessage::RequestError(err)) => {
            let error = format!(
              "upstream FETCH {} rejected: {:?}",
              relay_request_id, err.error_code
            );
            let _ = response_tx
              .take()
              .unwrap()
              .send(UpstreamFetchResponse::Rejected(err));
            // Tell the waiting gap loop now instead of letting it sit until its timeout.
            let _ = response_event_tx
              .send(UpstreamFetchEvent::Error(error))
              .await;
            return;
          }
          Ok(ControlMessage::RequestUpdate(_)) => {
            super::super::close_on_unexpected_request_update(&upstream_publisher, "FETCH");
            let error = format!(
              "upstream FETCH {} failed: publisher sent REQUEST_UPDATE",
              relay_request_id
            );
            let _ = response_tx
              .take()
              .unwrap()
              .send(UpstreamFetchResponse::Failed(error.clone()));
            let _ = response_event_tx
              .send(UpstreamFetchEvent::Error(error))
              .await;
            return;
          }
          Ok(other) => {
            warn!(
              "Unexpected {:?} on upstream fetch stream {}",
              other.get_type(),
              relay_request_id
            );
            return;
          }
          Err(e) => {
            trace!("Upstream fetch stream {} closed: {:?}", relay_request_id, e);
            return;
          }
        }
        // Stays parked until the publisher closes the request stream. The relay sent
        // this FETCH, so the publisher has no business updating it.
        if let Ok(ControlMessage::RequestUpdate(_)) = upstream.next_message().await {
          super::super::close_on_unexpected_request_update(&upstream_publisher, "FETCH");
        }
      };
      let end = tokio::select! {
        biased;
        reason = &mut cancel_rx => Err(reason.unwrap_or(UpstreamRequestCancellation::Cancelled)),
        _ = response => Ok(false),
        _ = response_event_tx.closed() => Ok(true),
      };
      if let Err(reason) = end {
        if let Some(response_tx) = response_tx.take() {
          let response = match reason {
            UpstreamRequestCancellation::MalformedTrack => UpstreamFetchResponse::MalformedTrack,
            UpstreamRequestCancellation::Cancelled => {
              UpstreamFetchResponse::Failed(format!("upstream FETCH {relay_request_id} cancelled"))
            }
          };
          let _ = response_tx.send(response);
        }
        upstream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
      } else if end == Ok(true) && closed_cleanly.load(Ordering::Acquire) {
        // The whole response arrived, so the request is closed rather than cancelled, and
        // a publisher that answers by closing its own side is given a moment to do so.
        upstream.finish().await;
        let _ = tokio::time::timeout(
          context.server_config.upstream_fetch_timeout,
          upstream.next_message(),
        )
        .await;
      } else if end == Ok(true) {
        upstream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
      } else {
        // The request stream may close before the fetch stream has delivered everything,
        // so the fetch is remembered until its reader is done.
        drop(response_tx);
        tokio::select! {
          _ = response_event_tx.closed() => {}
          _ = &mut cancel_rx => upstream.reset_and_stop(StreamResetCode::Cancelled.to_u64()),
        }
      }
      forget_upstream_fetch(&context, &upstream_publisher, relay_request_id).await;
      request_track
        .unregister_upstream_request(relay_request_id)
        .await;
    }
  });

  Some((
    relay_request_id,
    publisher,
    closed_cleanly,
    upstream_rx,
    response_rx,
  ))
}

async fn forget_upstream_fetch(
  context: &SessionContext,
  publisher: &MOQTClient,
  relay_request_id: u64,
) {
  context
    .upstream_requests
    .remove_fetch(relay_request_id)
    .await;
  publisher
    .outgoing_fetch_requests
    .write()
    .await
    .remove(&relay_request_id);
}

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

pub(crate) async fn handle_malformed_upstream_fetch(
  context: &SessionContext,
  track: &Arc<Track>,
  publisher: &MOQTClient,
) {
  let (full_track_name, remove_track) = {
    let _handover = track.lock_handover().await;
    publisher
      .mark_track_malformed(track.full_track_name.clone())
      .await;
    cancel_downstream_fetches(context, &track.full_track_name).await;
    track
      .cancel_for_malformed_publisher(publisher.connection_id)
      .await;
    let still_served =
      crate::server::message_handlers::subscribe_handler::upstream::remove_upstream_publisher(
        track,
        publisher.connection_id,
        context,
      )
      .await;
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
}
