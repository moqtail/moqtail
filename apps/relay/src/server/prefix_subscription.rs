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

//! What SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS share.
//!
//! Both name a namespace prefix, both hold their request stream open for as long as
//! the subscription lives, and both are updated and cancelled the same way. What they
//! produce differs -- one sends NAMESPACE and NAMESPACE_DONE, the other PUBLISH -- and
//! that part stays in `subscribe_namespace_handler` and `subscribe_tracks_handler`
//! respectively, alongside the publisher-side fan-out that feeds it.
//!
//! Everything here is written once and parameterised by `SubscribeKind`, which selects
//! the overlap space to check and the registry entry to act on. The two kinds never
//! conflict with each other.

use crate::server::client::MOQTClient;
use crate::server::session_context::{PendingRequest, SessionContext};
use crate::server::track_manager::SubscribeKind;
use core::result::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::request_ok::RequestOk;
use moqtail::model::error::TerminationCode;
use moqtail::model::error::{RequestErrorCode, StreamResetCode};
use moqtail::model::parameter::message_parameter::{
  MessageParameter, apply_message_parameter_update,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{info, warn};

/// The relay will not enumerate a namespace prefix broader than this many fields.
pub(crate) const MAX_NAMESPACE_PREFIX_FIELDS: usize = 32;

/// A NAMESPACE_TOO_LARGE error if the prefix exceeds what the relay will
/// enumerate, otherwise `None`.
pub(crate) fn oversized_namespace_error(prefix: &Tuple) -> Option<RequestError> {
  if prefix.fields.len() > MAX_NAMESPACE_PREFIX_FIELDS {
    Some(RequestError::new(
      RequestErrorCode::NamespaceTooLarge,
      0,
      ReasonPhrase::try_new("Namespace prefix is too large".to_string()).unwrap(),
    ))
  } else {
    None
  }
}

/// Remove the subscription when the subscriber closes or resets the bi-stream.
pub async fn cancel(client: Arc<MOQTClient>, request_id: u64, context: &Arc<SessionContext>) {
  let target = {
    let mut map = client.inbound_requests.write().await;
    match map.remove(&request_id) {
      Some(PendingRequest::SubscribeNamespace { message, .. }) => {
        Some((message.track_namespace_prefix, SubscribeKind::Namespace))
      }
      Some(PendingRequest::SubscribeTracks { message, .. }) => {
        Some((message.track_namespace_prefix, SubscribeKind::Tracks))
      }
      _ => None,
    }
  };
  if let Some((prefix, kind)) = target {
    context
      .track_manager
      .remove_namespace_subscriber_by_prefix(&prefix, client.connection_id, kind)
      .await;
    info!(
      "Cancelled {:?} subscription for prefix {:?} (connection {})",
      kind, prefix, client.connection_id
    );
  }
}

/// The prefix subscription a REQUEST_UPDATE targets: the prefix it currently holds
/// and which kind it is.
async fn target_subscription(
  client: &Arc<MOQTClient>,
  request_id: u64,
) -> Option<(Tuple, SubscribeKind)> {
  let map = client.inbound_requests.read().await;
  match map.get(&request_id) {
    Some(PendingRequest::SubscribeNamespace { message, .. }) => Some((
      message.track_namespace_prefix.clone(),
      SubscribeKind::Namespace,
    )),
    Some(PendingRequest::SubscribeTracks { message, .. }) => Some((
      message.track_namespace_prefix.clone(),
      SubscribeKind::Tracks,
    )),
    _ => None,
  }
}

/// The new prefix a REQUEST_UPDATE asks the subscription to move to, if any.
fn requested_prefix(parameters: &[MessageParameter]) -> Option<Tuple> {
  parameters.iter().find_map(|p| match p {
    MessageParameter::TrackNamespacePrefix { prefix } => Some(prefix.clone()),
    _ => None,
  })
}

/// The error to answer a prefix move with, or `None` to let it through. The
/// subscription's own entry is excluded from the overlap check, since the prefix it
/// is vacating always overlaps whatever replaces it.
async fn prefix_move_rejection(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  current_prefix: &Tuple,
  new_prefix: &Tuple,
  kind: SubscribeKind,
) -> Option<RequestError> {
  if let Some(err) = oversized_namespace_error(new_prefix) {
    warn!(
      "REQUEST_UPDATE prefix has {} fields, maximum is {}",
      new_prefix.fields.len(),
      MAX_NAMESPACE_PREFIX_FIELDS
    );
    return Some(err);
  }

  let existing_prefix = context
    .track_manager
    .find_overlapping_namespace_subscription(
      client.connection_id,
      new_prefix,
      kind,
      Some(current_prefix),
    )
    .await?;

  warn!(
    "REQUEST_UPDATE prefix overlap: new={new_prefix:?} conflicts with existing={existing_prefix:?}"
  );
  Some(RequestError::new(
    RequestErrorCode::PrefixOverlap,
    0,
    ReasonPhrase::try_new("Namespace prefix overlaps with existing subscription".to_string())
      .unwrap(),
  ))
}

/// Applies an accepted REQUEST_UPDATE to the stored request and to the registry,
/// moving the subscription when the update asked for a new prefix. The prefix is the
/// registry key that the ongoing fan-out and its suffixes are computed from, so moving
/// the entry is what makes a new prefix take effect.
async fn apply_subscription_update(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  request_id: u64,
  current_prefix: &Tuple,
  new_prefix: Option<Tuple>,
  kind: SubscribeKind,
  parameters: Vec<MessageParameter>,
) {
  {
    let mut map = client.inbound_requests.write().await;
    let stored = match map.get_mut(&request_id) {
      Some(PendingRequest::SubscribeNamespace { message, .. }) => {
        Some((&mut message.parameters, &mut message.track_namespace_prefix))
      }
      Some(PendingRequest::SubscribeTracks { message, .. }) => {
        Some((&mut message.parameters, &mut message.track_namespace_prefix))
      }
      _ => None,
    };
    if let Some((stored_parameters, stored_prefix)) = stored {
      apply_message_parameter_update(stored_parameters, parameters.clone());
      if let Some(new_prefix) = &new_prefix {
        *stored_prefix = new_prefix.clone();
      }
    }
  }

  context
    .track_manager
    .update_namespace_subscription_parameters(current_prefix, client.connection_id, parameters)
    .await;

  if let Some(new_prefix) = new_prefix {
    info!(
      "Moving {:?} subscription of connection {} from {:?} to {:?}",
      kind, client.connection_id, current_prefix, new_prefix
    );
    context
      .track_manager
      .rekey_namespace_subscriber(current_prefix, new_prefix, client.connection_id, kind)
      .await;
  }
}

/// Handle a REQUEST_UPDATE arriving on a prefix subscription's own request stream.
pub async fn handle(
  client: Arc<MOQTClient>,
  handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
  opening_request_id: Option<u64>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::RequestUpdate(m) => {
      let update_msg = *m;
      let Some(existing_req_id) = opening_request_id else {
        return Err(TerminationCode::ProtocolViolation);
      };

      let Some((current_prefix, kind)) = target_subscription(&client, existing_req_id).await else {
        warn!(
          "REQUEST_UPDATE for prefix subscription request {} cannot be applied; closing the stream",
          existing_req_id
        );
        handler.reset(StreamResetCode::Cancelled.to_u64());
        return Ok(());
      };

      // A rejected prefix move leaves the parameters that arrived with it unapplied.
      let new_prefix = requested_prefix(&update_msg.parameters);
      if let Some(new_prefix) = &new_prefix
        && let Some(err) =
          prefix_move_rejection(&client, &context, &current_prefix, new_prefix, kind).await
      {
        handler
          .send(&ControlMessage::RequestError(Box::new(err)))
          .await?;
        return Ok(());
      }

      info!("Processing {kind:?} update for prefix: {current_prefix:?}");

      apply_subscription_update(
        &client,
        &context,
        existing_req_id,
        &current_prefix,
        new_prefix,
        kind,
        update_msg.parameters,
      )
      .await;

      let ok_msg = RequestOk::new(vec![]);
      handler
        .send(&ControlMessage::RequestOk(Box::new(ok_msg)))
        .await?;
    }

    _ => {
      warn!(
        "Unexpected message in prefix_subscription handler: {:?}",
        msg
      );
    }
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::common::tuple::TupleField;

  #[test]
  fn oversized_namespace_prefix_yields_namespace_too_large() {
    let big = Tuple {
      fields: (0..=MAX_NAMESPACE_PREFIX_FIELDS)
        .map(|i| TupleField::from_utf8(&i.to_string()))
        .collect(),
    };
    let err = oversized_namespace_error(&big).expect("over-long prefix must be rejected");
    assert_eq!(err.error_code, RequestErrorCode::NamespaceTooLarge);
    assert_eq!(u64::from(err.error_code), 0x31);
  }

  #[test]
  fn normal_namespace_prefix_is_accepted() {
    let small = Tuple {
      fields: vec![
        TupleField::from_utf8("moqtail"),
        TupleField::from_utf8("demo"),
      ],
    };
    assert!(oversized_namespace_error(&small).is_none());
  }

  #[test]
  fn a_prefix_move_is_read_from_the_update_parameters() {
    let prefix = Tuple::from_utf8_path("meet/room1");
    let parameters = vec![
      MessageParameter::new_forward(true),
      MessageParameter::new_track_namespace_prefix(prefix.clone()),
    ];
    assert_eq!(requested_prefix(&parameters), Some(prefix));

    // An update that carries no prefix leaves the subscription where it is.
    assert_eq!(
      requested_prefix(&[MessageParameter::new_forward(true)]),
      None
    );
  }
}
