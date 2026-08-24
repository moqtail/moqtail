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
use crate::server::prefix_subscription::{MAX_NAMESPACE_PREFIX_FIELDS, oversized_namespace_error};
use crate::server::session_context::{PendingRequest, SessionContext};
use crate::server::track_manager::SubscribeKind;
use core::result::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::namespace::Namespace;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::request_ok::RequestOk;
use moqtail::model::control::subscribe_namespace::SubscribeNamespace;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::TerminationCode;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

/// Handle an incoming SUBSCRIBE_NAMESPACE on a dedicated bi-stream.
/// Called from `dispatch_request_stream_message()` in session.rs.
pub async fn handle_subscribe_namespace(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  sub_ns: Box<SubscribeNamespace>,
  context: Arc<SessionContext>,
  namespace_tx: UnboundedSender<ControlMessage>,
) -> Result<(), TerminationCode> {
  info!(
    "Received SubscribeNamespace message: {:?}",
    sub_ns.track_namespace_prefix
  );

  // An over-large namespace prefix is rejected with NAMESPACE_TOO_LARGE on the
  // request stream (not a session teardown).
  if let Some(err) = oversized_namespace_error(&sub_ns.track_namespace_prefix) {
    warn!(
      "SUBSCRIBE_NAMESPACE prefix has {} fields, maximum is {}",
      sub_ns.track_namespace_prefix.fields.len(),
      MAX_NAMESPACE_PREFIX_FIELDS
    );
    stream_handler
      .send(&ControlMessage::RequestError(Box::new(err)))
      .await?;
    return Ok(());
  }

  // Independent overlap space: only other SUBSCRIBE_NAMESPACE prefixes conflict.
  if let Some(existing_prefix) = context
    .track_manager
    .find_overlapping_namespace_subscription(
      client.connection_id,
      &sub_ns.track_namespace_prefix,
      SubscribeKind::Namespace,
      None,
    )
    .await
  {
    warn!(
      "SUBSCRIBE_NAMESPACE overlap: new={:?} conflicts with existing={:?}",
      sub_ns.track_namespace_prefix, existing_prefix
    );
    let err = RequestError::new(
      RequestErrorCode::PrefixOverlap,
      0,
      ReasonPhrase::try_new("Namespace prefix overlaps with existing subscription".to_string())
        .unwrap(),
    );
    stream_handler
      .send(&ControlMessage::RequestError(Box::new(err)))
      .await?;
    return Ok(());
  }

  // Store the subscription in TrackManager with the channel sender
  context
    .track_manager
    .add_namespace_subscriber(
      sub_ns.track_namespace_prefix.clone(),
      client.clone(),
      SubscribeKind::Namespace,
      sub_ns.parameters.clone(),
      namespace_tx,
    )
    .await;

  // Track in inbound_requests so cancel() can find it
  {
    let mut map = client.inbound_requests.write().await;
    map.insert(
      sub_ns.request_id,
      PendingRequest::SubscribeNamespace {
        client_connection_id: client.connection_id,
        original_request_id: sub_ns.request_id,
        message: (*sub_ns).clone(),
      },
    );
  }

  // Send REQUEST_OK on the bi-stream
  let ok = RequestOk::new(vec![]);
  stream_handler
    .send(&ControlMessage::RequestOk(Box::new(ok)))
    .await?;
  info!(
    "Sent RequestOk for SubscribeNamespace request_id: {}",
    sub_ns.request_id
  );

  // Discovery: send NAMESPACE for each matching announced namespace, and echo the
  // namespaces of tracks already published under the prefix.
  send_namespace_catchup(stream_handler, &context, &sub_ns.track_namespace_prefix).await?;

  Ok(())
}

/// Send NAMESPACE messages for a discovery subscription: matching announced
/// namespaces and the namespaces of tracks already published under the prefix.
async fn send_namespace_catchup(
  stream_handler: &mut ControlStreamHandler,
  context: &Arc<SessionContext>,
  prefix: &moqtail::model::common::tuple::Tuple,
) -> Result<(), TerminationCode> {
  let mut seen: std::collections::HashSet<moqtail::model::common::tuple::Tuple> =
    std::collections::HashSet::new();

  let matched_announcements = context
    .track_manager
    .get_announcements_by_prefix(prefix)
    .await;
  for ns in matched_announcements {
    if seen.insert(ns.clone())
      && let Some(suffix) = ns.suffix(prefix)
    {
      let ns_msg = ControlMessage::Namespace(Box::new(Namespace::new(suffix)));
      stream_handler.send(&ns_msg).await?;
    }
  }

  let matched_tracks = context
    .track_manager
    .get_tracks_and_publishes_by_namespace_prefix(prefix)
    .await;
  for (full_track_name, _track_arc, _publish) in matched_tracks {
    let ns = full_track_name.namespace.clone();
    if seen.insert(ns.clone())
      && let Some(suffix) = ns.suffix(prefix)
    {
      let ns_msg = ControlMessage::Namespace(Box::new(Namespace::new(suffix)));
      stream_handler.send(&ns_msg).await?;
    }
  }
  Ok(())
}
