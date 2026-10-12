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

//! The bidirectional request streams a peer opens. Each carries one request: it
//! opens with the request message, then carries that request's responses and
//! follow-ups until either side closes it.

use moqtail::model::{
  common::reason_phrase::ReasonPhrase,
  control::{control_message::ControlMessage, request_error::RequestError},
  error::{RequestErrorCode, StreamResetCode, TerminationCode},
};
use moqtail::transport::{
  connection::{TransportRecvStream, TransportSendStream},
  control_stream_handler::ControlStreamHandler,
};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{error, trace, warn};

use super::{
  client::MOQTClient,
  message_handlers::{self, MessageHandler},
  prefix_subscription,
  session::Session,
  session_context::SessionContext,
};

/// What a request stream serves. It decides which follow-ups the stream accepts and
/// what closing the stream cancels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestStreamKind {
  Subscribe,
  Fetch,
  PublishNamespace,
  /// SUBSCRIBE_NAMESPACE or SUBSCRIBE_TRACKS.
  PrefixSubscription,
  /// Closing the stream leaves nothing to cancel.
  Publish,
  /// Closing the stream leaves nothing to cancel, and nothing on it can be updated.
  TrackStatus,
}

impl RequestStreamKind {
  /// The kind of stream `msg` opens, or `None` if it cannot open one.
  fn of(msg: &ControlMessage) -> Option<Self> {
    match msg {
      ControlMessage::Subscribe(_) => Some(Self::Subscribe),
      ControlMessage::Fetch(_) => Some(Self::Fetch),
      ControlMessage::PublishNamespace(_) => Some(Self::PublishNamespace),
      ControlMessage::SubscribeNamespace(_) | ControlMessage::SubscribeTracks(_) => {
        Some(Self::PrefixSubscription)
      }
      ControlMessage::Publish(_) => Some(Self::Publish),
      ControlMessage::TrackStatus(_) => Some(Self::TrackStatus),
      _ => None,
    }
  }
}

/// Serves a request stream the peer opened, from its first message until it closes.
pub(crate) async fn accept(
  context: Arc<SessionContext>,
  send_stream: TransportSendStream,
  recv_stream: TransportRecvStream,
) {
  let client = match context.get_client().await {
    Some(c) => c,
    None => return,
  };
  let mut stream_handler =
    ControlStreamHandler::new(send_stream, recv_stream).with_peer_id(context.connection_id);

  let msg = match stream_handler.next_message().await {
    Ok(m) => m,
    Err(e) => {
      error!("Failed to read first message on request bi-stream: {:?}", e);
      return;
    }
  };

  // A request stream MUST begin with a First-marked type; anything else is a
  // protocol violation and closes the session.
  if !msg.get_type().is_first() {
    warn!(
      "Request bi-stream opened with non-First message {:?}",
      msg.get_type()
    );
    Session::close_session(
      context,
      TerminationCode::ProtocolViolation,
      "Request stream must begin with a First-marked message",
    );
    return;
  }

  // While draining, reject new requests with GOING_AWAY.
  if context.draining.load(Ordering::Relaxed) {
    warn!(
      "Rejecting new request {:?} while draining (GOING_AWAY)",
      msg.get_type()
    );
    let err = RequestError::new(
      RequestErrorCode::GoingAway,
      0,
      ReasonPhrase::try_new("Relay is draining".to_string()).unwrap(),
    );
    let _ = stream_handler
      .send(&ControlMessage::RequestError(Box::new(err)))
      .await;
    return;
  }

  // Load shedding: once the relay is serving its configured maximum of request
  // streams, reset new ones with EXCESSIVE_LOAD instead of serving them.
  let max_active = context.server_config.max_active_requests;
  if max_active > 0 && context.active_request_streams.load(Ordering::Relaxed) >= max_active {
    warn!(
      "Load shedding request stream {:?}: {} active >= max {}",
      msg.get_type(),
      context.active_request_streams.load(Ordering::Relaxed),
      max_active
    );
    stream_handler.reset(StreamResetCode::ExcessiveLoad.to_u64());
    return;
  }

  context
    .active_request_streams
    .fetch_add(1, Ordering::Relaxed);
  dispatch(client, stream_handler, msg, context.clone()).await;
  context
    .active_request_streams
    .fetch_sub(1, Ordering::Relaxed);
}

/// Routes a request stream by the request that opened it. Every request stream
/// passes through here with its Request ID in hand.
async fn dispatch(
  client: Arc<MOQTClient>,
  stream_handler: ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) {
  let (Some(kind), Some(request_id)) = (RequestStreamKind::of(&msg), msg.request_id()) else {
    warn!(
      "Unsupported message type {:?} on request bi-stream",
      msg.get_type()
    );
    Session::close_session(
      context,
      TerminationCode::ProtocolViolation,
      "Unsupported message type on request bi-stream",
    );
    return;
  };

  match kind {
    RequestStreamKind::PrefixSubscription => {
      serve_prefix_subscription(client, stream_handler, msg, request_id, context).await
    }
    _ => serve_request(kind, client, stream_handler, msg, request_id, context).await,
  }
}

/// A prefix subscription's stream carries the NAMESPACE and PUBLISH messages it
/// matches until the subscription ends or the subscriber closes the stream.
async fn serve_prefix_subscription(
  client: Arc<MOQTClient>,
  mut stream_handler: ControlStreamHandler,
  msg: ControlMessage,
  request_id: u64,
  context: Arc<SessionContext>,
) {
  let (namespace_tx, mut namespace_rx) = tokio::sync::mpsc::unbounded_channel::<ControlMessage>();

  if let Err(e) = MessageHandler::handle_prefix_subscription(
    client.clone(),
    &mut stream_handler,
    msg,
    context.clone(),
    namespace_tx,
  )
  .await
  {
    error!("prefix subscription handler error: {:?}", e);
    return;
  }

  loop {
    tokio::select! {
      biased;
      msg_opt = namespace_rx.recv() => {
        match msg_opt {
          Some(ns_msg) => {
            if let Err(e) = stream_handler.send(&ns_msg).await {
              warn!("Error writing NAMESPACE to bi-stream: {:?}", e);
              cancel(RequestStreamKind::PrefixSubscription, client, request_id, &context).await;
              return;
            }
          }
          None => return, // channel dropped, subscription ended
        }
      }
      incoming = stream_handler.next_message() => {
        match incoming {
          Ok(follow_up) => {
            if let Err((code, reason)) = handle_follow_up(
              RequestStreamKind::PrefixSubscription,
              &client,
              &mut stream_handler,
              follow_up,
              request_id,
              &context,
            )
            .await
            {
              Session::close_session(context, code, reason);
              return;
            }
          }
          Err(_) => {
            // FIN or RESET — subscriber cancelled
            cancel(RequestStreamKind::PrefixSubscription, client, request_id, &context).await;
            return;
          }
        }
      }
    }
  }
}

/// Request/response request types: the handler writes its immediate response on this
/// stream. A response channel registered under the Request ID lets deferred responses
/// (e.g. a SUBSCRIBE_OK fan-out produced elsewhere) land on this stream too. The loop
/// then also forwards follow-ups (UNSUBSCRIBE, REQUEST_UPDATE, FETCH_CANCEL) until the
/// peer closes the stream.
async fn serve_request(
  kind: RequestStreamKind,
  client: Arc<MOQTClient>,
  mut stream_handler: ControlStreamHandler,
  msg: ControlMessage,
  request_id: u64,
  context: Arc<SessionContext>,
) {
  let (response_tx, mut response_rx) = tokio::sync::mpsc::unbounded_channel();
  client
    .register_response_sender(request_id, response_tx)
    .await;

  if let Err(e) = MessageHandler::handle(
    client.clone(),
    &mut stream_handler,
    msg,
    context.clone(),
    Some(request_id),
  )
  .await
  {
    client.unregister_response_sender(request_id).await;
    Session::close_session(context, e, "Error handling request stream message");
    return;
  }

  loop {
    tokio::select! {
      biased;
      deferred = response_rx.recv() => {
        match deferred {
          Some(msg) => {
            if let Err(e) = stream_handler.send(&msg).await {
              warn!("Error writing response to request bi-stream: {:?}", e);
              break;
            }
          }
          None => break, // response channel closed
        }
      }
      incoming = stream_handler.next_message() => {
        match incoming {
          Ok(follow_up) => {
            if let Err((code, reason)) = handle_follow_up(
              kind,
              &client,
              &mut stream_handler,
              follow_up,
              request_id,
              &context,
            )
            .await
            {
              client.unregister_response_sender(request_id).await;
              Session::close_session(context, code, reason);
              return;
            }
          }
          Err(_) => {
            // FIN or RESET: the requester cancelled by closing the stream.
            trace!("Request bi-stream closed by peer");
            cancel(kind, client.clone(), request_id, &context).await;
            break;
          }
        }
      }
    }
  }
  client.unregister_response_sender(request_id).await;
}

/// Handles a message that follows the request on its stream. An error names what the
/// session is closed with.
async fn handle_follow_up(
  kind: RequestStreamKind,
  client: &Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  request_id: u64,
  context: &Arc<SessionContext>,
) -> Result<(), (TerminationCode, &'static str)> {
  if let Some(reason) = rejected_follow_up(kind, &msg) {
    warn!("{} ({:?})", reason, msg.get_type());
    return Err((TerminationCode::ProtocolViolation, reason));
  }

  MessageHandler::handle(
    client.clone(),
    stream_handler,
    msg,
    context.clone(),
    Some(request_id),
  )
  .await
  .map_err(|e| (e, "Error handling request stream follow-up"))
}

/// Why `msg` may not follow the request that opened a `kind` stream, if it may not.
/// The peer opened the stream, so it sent the request and may update it, unless the
/// request is one that cannot be updated at all.
fn rejected_follow_up(kind: RequestStreamKind, msg: &ControlMessage) -> Option<&'static str> {
  let is_update = matches!(msg, ControlMessage::RequestUpdate(_));
  match kind {
    // A REQUEST_UPDATE is the only thing that follows a prefix subscription.
    RequestStreamKind::PrefixSubscription if !is_update => {
      Some("Unexpected message on request bi-stream")
    }
    RequestStreamKind::TrackStatus if is_update => {
      Some("Unexpected REQUEST_UPDATE on request bi-stream")
    }
    _ => None,
  }
}

/// Cancels what the request still holds once its stream has closed under it.
async fn cancel(
  kind: RequestStreamKind,
  client: Arc<MOQTClient>,
  request_id: u64,
  context: &Arc<SessionContext>,
) {
  match kind {
    RequestStreamKind::Subscribe => {
      message_handlers::subscribe_handler::cancel_subscription(client, request_id, context).await;
    }
    RequestStreamKind::Fetch => {
      message_handlers::fetch_handler::cancel_fetch(client, request_id).await;
    }
    RequestStreamKind::PublishNamespace => {
      message_handlers::publish_namespace_handler::cancel(client, request_id, context).await;
    }
    RequestStreamKind::PrefixSubscription => {
      prefix_subscription::cancel(client, request_id, context).await;
    }
    RequestStreamKind::Publish | RequestStreamKind::TrackStatus => {}
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::control::{request_ok::RequestOk, request_update::RequestUpdate};

  fn update() -> ControlMessage {
    ControlMessage::RequestUpdate(Box::new(RequestUpdate::new(0, vec![])))
  }

  #[test]
  fn request_update_follows_every_updatable_request() {
    for kind in [
      RequestStreamKind::Subscribe,
      RequestStreamKind::Fetch,
      RequestStreamKind::PublishNamespace,
      RequestStreamKind::PrefixSubscription,
      RequestStreamKind::Publish,
    ] {
      assert_eq!(rejected_follow_up(kind, &update()), None, "{kind:?}");
    }
  }

  #[test]
  fn request_update_on_track_status_is_rejected() {
    assert!(rejected_follow_up(RequestStreamKind::TrackStatus, &update()).is_some());
  }

  #[test]
  fn only_request_update_follows_a_prefix_subscription() {
    let ok = ControlMessage::RequestOk(Box::new(RequestOk::new(vec![])));
    assert!(rejected_follow_up(RequestStreamKind::PrefixSubscription, &ok).is_some());
    assert_eq!(rejected_follow_up(RequestStreamKind::Subscribe, &ok), None);
  }
}
