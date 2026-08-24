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
use crate::server::client::switch_context::SwitchStatus;
use crate::server::message_handlers::parameters;
use crate::server::session::Session;
use crate::server::session_context::{PendingRequest, SessionContext};
use crate::server::track::{Track, TrackOrigin, TrackStatus};
use core::result::Result;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::request_ok::RequestOk;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::StreamResetCode;
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::message_parameter::{
  MessageParameter, MessageParameterVecExt, apply_message_parameter_update,
};
use moqtail::model::property::track_property::has_unsupported_mandatory;
use moqtail::model::{
  common::reason_phrase::ReasonPhrase, control::control_message::ControlMessage,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::SubscribeRequest;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

async fn add_subscription(
  subscribe: Subscribe,
  track: &Track,
  subscriber: Arc<MOQTClient>,
  is_switch: bool,
) -> bool {
  match track
    .add_subscription(subscriber.clone(), subscribe, is_switch)
    .await
  {
    Ok(subscription) => {
      subscriber
        .subscriptions
        .add_subscription(track.full_track_name.clone(), Arc::downgrade(&subscription))
        .await;
      true
    }
    Err(_) => false, // error already logged in add_subscription and it means that subscription already exists
  }
}

/// Forward a SUBSCRIBE to the upstream publisher on its own bidirectional request
/// stream, then read the response (and follow-ups) on that stream and dispatch
/// them so the result fans out to the downstream subscribers.
async fn forward_subscribe_upstream(
  publisher: Arc<MOQTClient>,
  new_sub: Subscribe,
  context: Arc<SessionContext>,
) {
  let relay_request_id = new_sub.request_id;
  upstream_subscribe_exchange(publisher, new_sub, context.clone()).await;

  // The entry exists to resolve this publisher's response against, and the exchange
  // has ended one way or another: answered, declined, timed out, cancelled, or never
  // sent. Dropping it here covers every exit inside, several of which return early.
  context
    .relay_pending_requests
    .write()
    .await
    .remove(&relay_request_id);
}

async fn upstream_subscribe_exchange(
  publisher: Arc<MOQTClient>,
  new_sub: Subscribe,
  context: Arc<SessionContext>,
) {
  let (send, recv) = match publisher.connection.open_bi().await {
    Ok(streams) => streams,
    Err(e) => {
      error!("Failed to open upstream subscribe stream: {:?}", e);
      return;
    }
  };
  // The response returns on this stream, so correlate it by the relay request id
  // we sent upstream rather than a field in the response.
  let relay_request_id = new_sub.request_id;
  let full_track_name = new_sub.get_full_track_name();
  let mut upstream = ControlStreamHandler::new(send, recv).with_peer_id(publisher.connection_id);
  if let Err(e) = upstream
    .send(&ControlMessage::Subscribe(Box::new(new_sub)))
    .await
  {
    error!("Failed to send upstream SUBSCRIBE: {:?}", e);
    return;
  }

  // Register a cancel signal on the track so the last downstream unsubscribe
  // resets this upstream stream and the publisher observes CANCELLED.
  let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
  if let Some(track) = context.track_manager.get_track(&full_track_name).await {
    track
      .read()
      .await
      .upstream_subscribe_cancellers
      .lock()
      .await
      .push(cancel_tx);
  }

  // A publisher owes exactly one SUBSCRIBE_OK or REQUEST_ERROR. One that stays
  // connected and sends neither would otherwise hold the subscriber's request stream
  // open forever, and — where several publishers were asked — keep the others' answers
  // from ever being acted on. The deadline only covers that first response; once it
  // arrives the stream lives as long as the subscription does.
  let answer_deadline = tokio::time::sleep(context.server_config.upstream_subscribe_timeout);
  tokio::pin!(answer_deadline);
  let mut answered = false;

  loop {
    tokio::select! {
      biased;
      _ = &mut cancel_rx => {
        info!("Downstream unsubscribed; resetting upstream subscribe stream (CANCELLED)");
        // Break and tear the stream down after the loop: `reset_and_stop`
        // consumes `upstream`, which can't be moved inside the select while the
        // `next_message()` branch borrows it. Reset the send half and stop the
        // recv half with the same CANCELLED code, so the publisher sees a
        // coherent cancellation on both halves rather than a STOP_SENDING(0)
        // (InternalError) emitted when the handler is dropped.
        break;
      }
      _ = &mut answer_deadline, if !answered => {
        warn!(
          "Publisher {} did not answer the SUBSCRIBE for {:?} in time; treating it as declined",
          publisher.connection_id, full_track_name
        );
        let err = RequestError::new(
          RequestErrorCode::Timeout,
          0,
          ReasonPhrase::try_new("Publisher did not answer the subscribe".to_string()).unwrap(),
        );
        end_upstream_subscription(
          relay_request_id,
          publisher.connection_id,
          &full_track_name,
          err,
          PublishDoneStatusCode::InternalError,
          context.clone(),
        )
        .await;
        break;
      }
      msg = upstream.next_message() => {
        answered = true;
        match msg {
          Ok(ControlMessage::SubscribeOk(m)) => {
            if let Err(e) =
              handle_subscribe_ok_message(publisher.clone(), relay_request_id, *m, context.clone())
                .await
            {
              error!("Error handling upstream SubscribeOk: {:?}", e);
              return;
            }
          }
          Ok(ControlMessage::RequestError(m)) => {
            let status = publish_done_status_for(m.error_code);
            end_upstream_subscription(
              relay_request_id,
              publisher.connection_id,
              &full_track_name,
              *m,
              status,
              context.clone(),
            )
            .await;
            return;
          }
          Ok(ControlMessage::PublishDone(m)) => {
            // Upstream is done for this track; relay its PUBLISH_DONE (status and
            // reason verbatim) to the downstream subscribers. It usually arrives
            // coalesced with the upstream stream FIN.
            info!(
              "Upstream PUBLISH_DONE for {:?}: status={:?} reason={}",
              full_track_name,
              m.status_code,
              m.reason_phrase.as_str()
            );
            if let Some(track) = context.track_manager.get_track(&full_track_name).await
              && let Err(e) = track
                .read()
                .await
                .notify_publish_done(m.status_code, m.reason_phrase.as_str().to_string())
                .await
            {
              error!("Failed to relay upstream PUBLISH_DONE downstream: {:?}", e);
            }
            return;
          }
          Ok(other) => {
            warn!("Unexpected {:?} on upstream subscribe stream", other.get_type());
            let err = RequestError::new(
              RequestErrorCode::InternalError,
              0,
              ReasonPhrase::try_new("Unexpected message on upstream subscription stream".to_string()).unwrap(),
            );
            // Nothing about the track changed; the pipeline misbehaved.
            end_upstream_subscription(
              relay_request_id,
              publisher.connection_id,
              &full_track_name,
              err,
              PublishDoneStatusCode::InternalError,
              context.clone(),
            )
            .await;
            return;
          }
          Err(code) => {
            warn!("Upstream subscribe stream closed with error; resetting: {:?}", code);
            let err = RequestError::new(
              RequestErrorCode::InternalError,
              0,
              ReasonPhrase::try_new("Upstream subscription failed".to_string()).unwrap(),
            );
            // The upstream stream is gone, so the track is no longer being published.
            end_upstream_subscription(
              relay_request_id,
              publisher.connection_id,
              &full_track_name,
              err,
              PublishDoneStatusCode::TrackEnded,
              context.clone(),
            )
            .await;
            return;
          }
        }
      }
    }
  }

  upstream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
}

/// The PUBLISH_DONE status that carries an upstream REQUEST_ERROR downstream. Only a few
/// of the two registries line up; the rest are a generic failure as far as a subscriber
/// is concerned.
fn publish_done_status_for(error_code: RequestErrorCode) -> PublishDoneStatusCode {
  match error_code {
    RequestErrorCode::Unauthorized => PublishDoneStatusCode::Unauthorized,
    RequestErrorCode::GoingAway => PublishDoneStatusCode::GoingAway,
    RequestErrorCode::MalformedTrack => PublishDoneStatusCode::MalformedTrack,
    RequestErrorCode::ExcessiveLoad => PublishDoneStatusCode::ExcessiveLoad,
    RequestErrorCode::DoesNotExist => PublishDoneStatusCode::TrackEnded,
    _ => PublishDoneStatusCode::InternalError,
  }
}

/// Ends every downstream subscription for a track whose upstream subscription failed.
///
/// Which message ends it depends on how far the downstream request got. One still
/// waiting for its answer is answered with REQUEST_ERROR. One already accepted has had
/// its single response, and only PUBLISH_DONE can end it — a second response on that
/// stream would be a protocol violation.
async fn end_upstream_subscription(
  relay_request_id: u64,
  publisher_connection_id: usize,
  full_track_name: &FullTrackName,
  error: RequestError,
  status_code: PublishDoneStatusCode,
  context: Arc<SessionContext>,
) {
  let track = context.track_manager.get_track(full_track_name).await;
  let (confirmed, others_may_still_accept) = match &track {
    Some(track) => {
      let track = track.read().await;
      let confirmed = matches!(*track.status.read().await, TrackStatus::Confirmed { .. });
      // This publisher has answered; whatever remains counted has not.
      let remaining = track
        .pending_upstream_subscribe_count
        .fetch_sub(1, Ordering::SeqCst)
        .saturating_sub(1);
      (confirmed, remaining > 0)
    }
    None => (false, false),
  };

  if !confirmed {
    // `handle_subscribe_error_message` marks the whole track Rejected and answers the
    // subscriber's request stream, ending the subscription for everyone. One publisher
    // declining is not grounds for that while others may still accept, and the stream
    // takes exactly one response either way. Every publisher answers or times out, so
    // the last one to do so reaches this with nothing outstanding and decides.
    if others_may_still_accept {
      info!(
        "A publisher declined {:?}; others have yet to answer, so the subscriber waits",
        full_track_name
      );
      return;
    }
    let _ = handle_subscribe_error_message(relay_request_id, error, context).await;
    return;
  }

  // The track was already accepted, so this publisher's failure ends its own upstream
  // subscription rather than the request. Only when it was the last publisher serving
  // the track is there nothing left to deliver, and the subscribers are told it is
  // over; while another still serves it they carry on and must not hear PUBLISH_DONE.
  let Some(track) = track else {
    return;
  };
  let still_served = {
    let track = track.read().await;
    let mut aliases = track.publisher_aliases.write().await;
    aliases.remove(&publisher_connection_id);
    !aliases.is_empty()
  };

  if still_served {
    info!(
      "Upstream subscription for {:?} from publisher {} failed; other publishers still serve it",
      full_track_name, publisher_connection_id
    );
    return;
  }

  info!(
    "Last upstream subscription for {:?} failed after acceptance; ending downstream with PUBLISH_DONE",
    full_track_name
  );
  if let Err(e) = track
    .read()
    .await
    .notify_publish_done(status_code, error.reason_phrase.as_str().to_string())
    .await
  {
    error!("Failed to end downstream subscriptions: {:?}", e);
  }
}

async fn handle_subscribe_message(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  sub: Subscribe,
  context: Arc<SessionContext>,
  is_switch: bool,
) -> Result<(), TerminationCode> {
  info!("received Subscribe message: {:?}", sub);
  let track_namespace = sub.track_namespace.clone();
  let full_track_name = sub.get_full_track_name();

  // Reserved namespaces are resolved locally and never forwarded upstream.
  if let Some(reason) =
    crate::server::utils::reserved_namespace_rejection(&track_namespace, &sub.track_name)
  {
    info!("Rejecting SUBSCRIBE for reserved namespace: {}", reason);
    let err = RequestError::new(
      RequestErrorCode::DoesNotExist,
      0,
      ReasonPhrase::try_new(reason.to_string()).unwrap(),
    );
    stream_handler.send_impl(&err).await.unwrap();
    return Ok(());
  }

  // Every publisher of the exact Track, plus every publisher that announced a namespace
  // it falls under. A SUBSCRIBE goes to all of them, not to whichever matched first.
  let publishers = {
    debug!("trying to get the publishers");
    context
      .client_manager
      .get_publishers_for_track(&full_track_name)
      .await
  };

  if publishers.is_empty() {
    info!(
      "no publisher found for track namespace: {:?}",
      track_namespace
    );
    // Dump what the relay actually had registered at the miss, so the sent
    // full track name / namespace can be compared against live registrations.
    info!(
      "SUBSCRIBE miss: sent full_track_name={:?} namespace={:?}; registrations:{}",
      full_track_name,
      track_namespace,
      context.client_manager.dump_registrations().await
    );
    // send RequestError
    let subscribe_error = RequestError::new(
      RequestErrorCode::DoesNotExist,
      0, //TODO: Maybe decide on another retry interval?
      ReasonPhrase::try_new("Unknown track namespace".to_string()).unwrap(),
    );
    stream_handler.send_impl(&subscribe_error).await.unwrap();
    return Ok(());
  };

  for publisher in &publishers {
    publisher.add_subscriber(context.connection_id).await;
  }

  info!(
    "Subscriber ({}) added to {} publisher(s) for {:?}",
    context.connection_id,
    publishers.len(),
    full_track_name
  );

  let original_request_id = sub.request_id;

  // Atomic get-or-create: first subscriber creates, subsequent ones find existing
  let (track_arc, is_creator) = context
    .track_manager
    .get_or_create_track(&full_track_name, |relay_track_id| {
      Track::new(
        relay_track_id,
        full_track_name.clone(),
        context.server_config,
        TrackStatus::Pending,
        TrackOrigin::Subscribe,
      )
    })
    .await;

  let track = track_arc.read().await;

  // An endpoint may hold only one subscription per track in a given role. A SWITCH is
  // the exception: it deliberately reuses the existing subscription, and the failure
  // here is how it hands over.
  if !add_subscription(sub.clone(), &track, client.clone(), is_switch).await && !is_switch {
    drop(track);
    info!(
      "Rejecting SUBSCRIBE from {} for {:?}: already subscribed",
      context.connection_id, &full_track_name
    );
    let err = RequestError::new(
      RequestErrorCode::DuplicateSubscription,
      0,
      ReasonPhrase::try_new("already subscribed to this track".to_string()).unwrap(),
    );
    stream_handler.send_impl(&err).await.unwrap();
    return Ok(());
  }

  let res: Result<(), TerminationCode> = if is_creator {
    // First subscriber for this track: forward Subscribe to publisher
    info!(
      "First subscriber for track {:?}, forwarding to publisher",
      &full_track_name
    );

    // Counted before any request goes out, since a publisher can answer while the rest
    // are still being sent.
    {
      let track = track_arc.read().await;
      track
        .pending_upstream_subscribe_count
        .store(publishers.len(), Ordering::SeqCst);
    }

    for publisher in &publishers {
      let mut new_sub = sub.clone();
      new_sub.subscribe_parameters = parameters::upstream_subscribe();
      // Its own request id, which is what routes this publisher's response back.
      new_sub.request_id =
        Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

      // Store the relay subscribe request mapping before forwarding, so the
      // upstream response can be routed back to this subscription.
      // TODO: we need to add a timeout here or another loop to control expired requests
      let req = SubscribeRequest::new(
        original_request_id,
        context.connection_id,
        sub.clone(),
        Some(new_sub.clone()),
      );
      {
        let mut requests = context.relay_pending_requests.write().await;
        requests.insert(new_sub.request_id, PendingRequest::Subscribe(req.clone()));
      }
      info!(
        "forwarding SUBSCRIBE for {:?} to publisher {} as relay request {}",
        full_track_name, publisher.connection_id, new_sub.request_id
      );

      // Forward SUBSCRIBE upstream on its own bidirectional request stream and read
      // the response there, per the request-stream model.
      let publisher_fwd = publisher.clone();
      let context_fwd = context.clone();
      tokio::spawn(async move {
        forward_subscribe_upstream(publisher_fwd, new_sub, context_fwd).await;
      });
    }

    // Do NOT send SubscribeOk yet -- wait for publisher confirmation
    Ok(())
  } else {
    // Subsequent subscriber: track already exists
    let track = track_arc.read().await;
    let status = track.get_status().await;

    match status {
      TrackStatus::Confirmed {
        upstream_parameters,
      } => {
        info!(
          "Track confirmed, sending SubscribeOk to subscriber {}",
          client.connection_id
        );
        let cached_properties = { track.track_properties.read().await.clone() };
        let params =
          parameters::downstream_subscribe_ok(&upstream_parameters, track.largest_object().await);
        let subscribe_ok = moqtail::model::control::subscribe_ok::SubscribeOk::new(
          track.relay_track_id,
          params,
          cached_properties,
        );
        let sent = stream_handler.send_impl(&subscribe_ok).await;
        if sent.is_ok()
          && let Some(subscription) = track.get_subscription(client.connection_id).await
        {
          subscription.read().await.mark_alias_announced();
        }
        sent
      }
      TrackStatus::Pending => {
        info!(
          "Track pending, subscriber {} will wait for confirmation",
          client.connection_id
        );
        let mut pending = track.pending_subscribers.write().await;
        pending.push((sub.request_id, context.connection_id));
        Ok(())
      }
      TrackStatus::Rejected {
        error_code,
        reason_phrase,
      } => {
        info!(
          "Track rejected, sending RequestError to subscriber {}",
          client.connection_id
        );
        let subscribe_error = RequestError::new(
          error_code,
          0, //TODO: Maybe decide on another retry interval?
          reason_phrase,
        );
        stream_handler.send_impl(&subscribe_error).await
      }
    }
  };

  // A subscriber attaching to a PUBLISH-created track is what makes the relay
  // want Objects, so tell the publisher to start sending.
  if res.is_ok() {
    super::publish_handler::ensure_upstream_forwarding(&track_arc, &context).await;
  }

  // Store in client's subscribe requests on success
  if res.is_ok() {
    let mut requests = client.subscribe_requests.write().await;
    let orig_req = SubscribeRequest::new(original_request_id, context.connection_id, sub, None);
    requests.insert(original_request_id, orig_req.clone());

    // dual bookkeeping here, necessary evil
    let mut inbound = client.inbound_requests.write().await;
    inbound.insert(
      original_request_id,
      PendingRequest::Subscribe(orig_req.clone()),
    );

    debug!(
      "inserted request into client's subscribe requests: {:?}",
      orig_req
    );
  } else {
    error!("error in adding subscription: {:?}", res);
  }
  res
}

async fn handle_subscribe_ok_message(
  // The publisher that sent this SUBSCRIBE_OK; its connection id keys the track alias.
  publisher: Arc<MOQTClient>,
  // The relay request id we sent upstream; this stream identifies the request.
  request_id: u64,
  msg: moqtail::model::control::subscribe_ok::SubscribeOk,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  info!("received SubscribeOk message: {:?}", msg);

  // Look up the relay subscribe request from the unified map
  let sub_request = {
    let requests = context.relay_pending_requests.read().await;
    match requests.get(&request_id).cloned() {
      Some(PendingRequest::Subscribe(m)) => {
        info!("request id is verified: {:?}", request_id);
        m
      }
      Some(_) => {
        warn!(
          "request id matched but wrong type for SubscribeOk: {:?}",
          request_id
        );
        return Ok(());
      }
      None => {
        warn!("request id is not verified: {:?}", request_id);
        return Ok(());
      }
    }
  };

  // SUBSCRIBE_OK with an unsupported mandatory track property. Reuse the
  // RequestError fan-out to cancel and notify downstream subscribers.
  if has_unsupported_mandatory(&msg.track_properties) {
    warn!(
      "SubscribeOk for request {} carries an unsupported mandatory track property; rejecting",
      request_id
    );
    let reason = ReasonPhrase::try_new("Unsupported mandatory track property".to_string())
      .map_err(|_| TerminationCode::InternalError)?;
    let err = RequestError::new(RequestErrorCode::UnsupportedExtension, 0, reason);
    return handle_subscribe_error_message(request_id, err, context).await;
  }

  let full_track_name = sub_request.original_subscribe_request.get_full_track_name();

  // The track must already exist (pre-created in Subscribe handler)
  let track_arc = match context.track_manager.get_track(&full_track_name).await {
    Some(t) => t,
    None => {
      error!(
        "Track not found for SubscribeOk, this should not happen: {:?}",
        &full_track_name
      );
      return Ok(());
    }
  };

  // Confirm the track with publisher's metadata; capture relay_track_id for SubscribeOk messages
  let (relay_track_id, confirmed_now) = {
    let mut track = track_arc.write().await;
    let confirmed_now = track
      .confirm(
        publisher.connection_id,
        msg.track_alias,
        msg.subscribe_parameters.clone(),
        msg.track_properties.clone(),
      )
      .await;
    // This publisher has answered.
    track
      .pending_upstream_subscribe_count
      .fetch_sub(1, Ordering::SeqCst);
    (track.relay_track_id, confirmed_now)
  };

  // Register the publisher's alias for data stream routing. Every accepting publisher
  // needs this, whether or not it was the one that confirmed the track, or its Objects
  // arrive on a stream the relay cannot route.
  context
    .track_manager
    .add_track_alias(
      publisher.connection_id,
      msg.track_alias,
      full_track_name.clone(),
    )
    .await;

  // Only the publisher that confirmed the track answers downstream. The others are now
  // serving it too, but the subscribers were told once already and their request
  // streams take exactly one response.
  if !confirmed_now {
    info!(
      "Publisher {} also accepted {:?}; subscribers already answered",
      publisher.connection_id, full_track_name
    );
    return Ok(());
  }

  // What the relay advertises downstream is not what upstream sent: by the time this
  // runs the relay may already have seen a later Object than upstream knew of.
  let downstream_params = {
    let track = track_arc.read().await;
    parameters::downstream_subscribe_ok(&msg.subscribe_parameters, track.largest_object().await)
  };

  // Send SubscribeOk to the FIRST subscriber (the creator)
  {
    let subscriber = { context.client_manager.get(sub_request.requested_by).await };
    if let Some(subscriber) = subscriber {
      let cached_properties = {
        let track = track_arc.read().await;
        track.track_properties.read().await.clone()
      };
      let subscribe_ok = moqtail::model::control::subscribe_ok::SubscribeOk::new(
        relay_track_id,
        downstream_params.clone(),
        cached_properties,
      );
      info!(
        "sending SubscribeOk to creator subscriber: {:?}",
        subscriber.connection_id
      );
      let delivered = subscriber
        .send_response(
          sub_request.original_request_id,
          ControlMessage::SubscribeOk(Box::new(subscribe_ok)),
        )
        .await;
      if !delivered {
        warn!(
          "no request stream for creator subscriber request {}",
          sub_request.original_request_id
        );
      } else if let Some(subscription) = track_arc
        .read()
        .await
        .get_subscription(subscriber.connection_id)
        .await
      {
        subscription.read().await.mark_alias_announced();
      }
    } else {
      warn!(
        "creator subscriber not found: {:?}",
        sub_request.requested_by
      );
    }
  }

  // Send SubscribeOk to ALL pending subscribers
  {
    let track = track_arc.read().await;
    let pending = {
      let mut pending = track.pending_subscribers.write().await;
      std::mem::take(&mut *pending)
    };

    for (subscriber_request_id, subscriber_connection_id) in pending {
      let subscriber = { context.client_manager.get(subscriber_connection_id).await };
      if let Some(subscriber) = subscriber {
        let cached_properties = {
          let track = track_arc.read().await;
          track.track_properties.read().await.clone()
        };
        let subscribe_ok = moqtail::model::control::subscribe_ok::SubscribeOk::new(
          relay_track_id,
          downstream_params.clone(),
          cached_properties,
        );
        info!(
          "sending SubscribeOk to pending subscriber: {:?}",
          subscriber.connection_id
        );
        let delivered = subscriber
          .send_response(
            subscriber_request_id,
            ControlMessage::SubscribeOk(Box::new(subscribe_ok)),
          )
          .await;
        if !delivered {
          warn!(
            "no request stream for pending subscriber request {}",
            subscriber_request_id
          );
        } else if let Some(subscription) = track.get_subscription(subscriber.connection_id).await {
          subscription.read().await.mark_alias_announced();
        }
      }
    }
  }

  // Subscription was already added in the Subscribe handler,
  // so we do NOT call add_subscription again here.
  Ok(())
}

/// Cancel a subscription when its SUBSCRIBE request stream is reset or closed.
pub(crate) async fn cancel_subscription(
  client: Arc<MOQTClient>,
  request_id: u64,
  context: &Arc<SessionContext>,
) {
  // find the track alias by using the request id
  let full_track_name = {
    let requests = client.subscribe_requests.read().await;
    let request = requests.get(&request_id);
    if request.is_none() {
      warn!("request not found for request id: {:?}", request_id);
      return;
    }
    request
      .unwrap()
      .original_subscribe_request
      .get_full_track_name()
  }; // read lock dropped here

  // remove the subscription from the track
  let track_option = context.track_manager.get_track(&full_track_name).await;

  if let Some(track_lock) = track_option {
    let (is_last_subscriber, origin) = {
      let track = track_lock.read().await;
      track.remove_subscription(context.connection_id).await;
      // When the last subscriber goes away, reset the upstream subscribe streams so
      // the publishers observe the cancellation. One per publisher serving the track.
      let last = if track.subscriber_count().await == 0 {
        for cancel in track.upstream_subscribe_cancellers.lock().await.drain(..) {
          let _ = cancel.send(());
        }
        true
      } else {
        false
      };
      (last, track.origin)
    }; // track read lock dropped here

    // Only a SUBSCRIBE-created track is removed here: its upstream subscription
    // has just been cancelled, so its cached state is stale and the next
    // SUBSCRIBE must re-subscribe upstream rather than be answered from it. A
    // PUBLISH-created track has a publisher still pushing to it and outlives
    // any number of subscribers.
    if is_last_subscriber && origin == TrackOrigin::Subscribe {
      context.track_manager.remove_track(&full_track_name).await;
      info!(
        "Removed track {:?} after last subscriber left; next SUBSCRIBE will re-subscribe upstream",
        full_track_name
      );
    }
  } else {
    warn!(
      "Subscription cancel: Track {:?} already removed.",
      full_track_name
    );
  }

  // remove the subscription from the client
  client
    .subscriptions
    .remove_subscription(&full_track_name)
    .await;

  // Remove the request from the client's request map so it doesn't leak
  {
    let mut requests = client.subscribe_requests.write().await;
    requests.remove(&request_id);

    let mut inbound = client.inbound_requests.write().await;
    inbound.remove(&request_id);

    debug!(
      "Cleaned up client subscribe request {} on cancel",
      request_id
    );
  }
}

pub async fn handle_request_update(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  update_msg: moqtail::model::control::request_update::RequestUpdate,
  context: Arc<SessionContext>,
  existing_req_id: u64,
) -> Result<(), TerminationCode> {
  let full_track_name = {
    let mut client_requests = client.subscribe_requests.write().await;
    match client_requests.get_mut(&existing_req_id) {
      Some(req) => {
        apply_message_parameter_update(
          &mut req.original_subscribe_request.subscribe_parameters,
          update_msg.parameters.clone(),
        );
        req.original_subscribe_request.get_full_track_name()
      }
      None => {
        warn!(
          "RequestUpdate existing_request_id {} is not a valid Subscribe request for this client",
          existing_req_id
        );
        return Err(TerminationCode::ProtocolViolation);
      }
    }
  };

  {
    let mut inbound = client.inbound_requests.write().await;
    if let Some(PendingRequest::Subscribe(req)) = inbound.get_mut(&existing_req_id) {
      apply_message_parameter_update(
        &mut req.original_subscribe_request.subscribe_parameters,
        update_msg.parameters.clone(),
      );
    }
  }

  // 2. Get the track instance
  let track_lock = context.track_manager.get_track(&full_track_name).await;

  if track_lock.is_none() {
    warn!("Track not found for track name: {:?}", full_track_name);
    return Err(TerminationCode::ProtocolViolation);
  }

  let track_arc = track_lock.unwrap();

  // Apply the update, releasing the track/sub read locks before responding or
  // terminating so termination can take the track write lock.
  let update_result = {
    let track_guard = track_arc.read().await;
    match track_guard.get_subscription(client.connection_id).await {
      Some(subscription) => Some(
        subscription
          .read()
          .await
          .update_subscription(update_msg)
          .await,
      ),
      None => None,
    }
  };

  match update_result {
    Some(Ok(())) => {
      info!(
        "Subscription updated successfully for track: {:?}",
        full_track_name
      );
      // The update may have turned this subscriber's Forward State on, which is
      // the first thing wanting Objects from a PUBLISH-created track.
      super::publish_handler::ensure_upstream_forwarding(&track_arc, &context).await;
      let ok_msg = RequestOk::new(vec![]);
      let _ = stream_handler.send_impl(&ok_msg).await;
    }
    Some(Err(e)) => {
      error!(
        "Subscription update failed for track: {:?}, error: {:?}",
        full_track_name, e
      );

      let err_msg = RequestError::new(
        RequestErrorCode::InternalError,
        0,
        ReasonPhrase::try_new(format!("Update failed: {:?}", e))
          .unwrap_or_else(|_| ReasonPhrase::try_new("Update failed".to_string()).unwrap()),
      );
      let _ = stream_handler.send_impl(&err_msg).await;

      // A failed update terminates the subscription with PUBLISH_DONE(UPDATE_FAILED).
      let stream_count = match track_arc
        .read()
        .await
        .get_subscription(client.connection_id)
        .await
      {
        Some(sub) => sub.read().await.opened_stream_count(),
        None => 0,
      };
      let done = PublishDone::new(
        PublishDoneStatusCode::UpdateFailed,
        stream_count,
        ReasonPhrase::try_new("REQUEST_UPDATE failed".to_string()).unwrap(),
      );
      let _ = stream_handler.send_impl(&done).await;

      track_arc
        .write()
        .await
        .remove_subscription(client.connection_id)
        .await;
      client
        .subscriptions
        .remove_subscription(&full_track_name)
        .await;
    }
    None => {
      warn!(
        "No active subscription found for client {} on track {:?}",
        client.connection_id, full_track_name
      );

      let err_msg = RequestError::new(
        RequestErrorCode::DoesNotExist,
        0,
        ReasonPhrase::try_new("Subscription not found".to_string()).unwrap(),
      );
      let _ = stream_handler.send_impl(&err_msg).await;
    }
  }

  Ok(())
}
async fn handle_subscribe_error_message(
  // The relay request id we sent upstream; this stream identifies the request.
  request_id: u64,
  subscribe_error_message: RequestError,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  info!(
    "received RequestError message: {:?}",
    subscribe_error_message
  );
  let msg = subscribe_error_message;

  // Look up and remove the relay subscribe request from the unified map
  let sub_request = {
    let mut requests = context.relay_pending_requests.write().await;
    match requests.remove(&request_id) {
      Some(PendingRequest::Subscribe(m)) => m,
      Some(_) => {
        warn!("RequestError for mismatched request type: {:?}", request_id);
        return Ok(());
      }
      None => {
        warn!("RequestError for unknown request id: {:?}", request_id);
        return Ok(());
      }
    }
  };

  let full_track_name = sub_request.original_subscribe_request.get_full_track_name();

  // Mark track as Rejected (if it exists)
  let track_arc = context.track_manager.get_track(&full_track_name).await;
  if let Some(track_arc) = &track_arc {
    let track = track_arc.read().await;
    track
      .reject(msg.error_code, msg.reason_phrase.clone())
      .await;
  }

  // Send RequestError to the FIRST subscriber (the creator)
  {
    let subscriber = { context.client_manager.get(sub_request.requested_by).await };
    if let Some(subscriber) = subscriber {
      let subscribe_error = RequestError::new(
        msg.error_code,
        0, //TODO: Maybe decide on another retry interval?
        msg.reason_phrase.clone(),
      );
      subscriber
        .send_response(
          sub_request.original_request_id,
          ControlMessage::RequestError(Box::new(subscribe_error)),
        )
        .await;
    }
  }

  // Send RequestError to ALL pending subscribers
  if let Some(track_arc) = &track_arc {
    let track = track_arc.read().await;
    let pending = {
      let mut pending = track.pending_subscribers.write().await;
      std::mem::take(&mut *pending)
    };

    for (subscriber_request_id, subscriber_connection_id) in pending {
      let subscriber = { context.client_manager.get(subscriber_connection_id).await };
      if let Some(subscriber) = subscriber {
        let subscribe_error = RequestError::new(
          msg.error_code,
          0, //TODO: Maybe decide on another retry interval?
          msg.reason_phrase.clone(),
        );
        subscriber
          .send_response(
            subscriber_request_id,
            ControlMessage::RequestError(Box::new(subscribe_error)),
          )
          .await;
      }
    }
  }

  // Remove the pre-created track from TrackManager
  if track_arc.is_some() {
    let mut tracks = context.track_manager.tracks.write().await;
    tracks.remove(&full_track_name);
  }

  Ok(())
}

async fn handle_switch_message(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  switch_message: moqtail::model::control::switch::Switch,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  info!("received Switch message: {:?}", switch_message);

  // now different from a normal subscribe, we need to
  // check whether there is a related track to switch from
  let switch_from_track = {
    let requests = client.subscribe_requests.read().await;

    let req = requests.get(&switch_message.subscription_request_id);
    match req {
      Some(req) => {
        let track_name = req.original_subscribe_request.get_full_track_name();
        if let Some(track) = context.track_manager.get_track(&track_name).await {
          info!(
            "found old track request, original request id: {:?}",
            req.original_request_id
          );
          Some(track.clone())
        } else {
          warn!("old track not found for track name: {:?}", track_name);
          None
        }
      }
      None => None,
    }
  };

  if switch_from_track.is_none() {
    warn!(
      "no existing track found for switch subscription request id: {:?}",
      switch_message.subscription_request_id
    );
    return Err(TerminationCode::ProtocolViolation);
  }

  let switch_from_track_guard = switch_from_track.unwrap();

  let switch_from_track = switch_from_track_guard.read().await;

  if let Some(sub) = client
    .subscriptions
    .get_subscription(&switch_from_track.full_track_name)
    .await
  {
    if sub.upgrade().is_none() {
      warn!(
        "subscription weak reference is dead for track: {:?} subscriber: {}",
        switch_from_track.full_track_name, context.connection_id
      );
      return Err(TerminationCode::ProtocolViolation);
    }

    let mut is_active = false;
    if let Some(sub) = sub.upgrade() {
      let sub = sub.read().await;
      is_active = sub.is_active().await;
    }

    if !is_active {
      warn!(
        "subscription is not active for track: {:?} subscriber: {}",
        switch_from_track.full_track_name, context.connection_id
      );
      return Err(TerminationCode::ProtocolViolation);
    }
  } else {
    warn!(
      "no subscription found for track: {:?} subscriber: {}",
      switch_from_track.full_track_name, context.connection_id
    );
    return Err(TerminationCode::ProtocolViolation);
  }

  let mut switch_params: Vec<MessageParameter> = switch_message
    .subscribe_parameters
    .iter()
    .filter_map(|kvp| MessageParameter::deserialize(kvp).ok())
    .collect();

  switch_params.set_param(MessageParameter::new_forward(true)); // forward always true for switch

  let subscribe = Subscribe::new_latest_object(
    switch_message.request_id,
    switch_message.track_namespace.clone(),
    switch_message.track_name.clone(),
    switch_params,
  );

  let new_full_track_name = subscribe.get_full_track_name();

  if let Err(e) = handle_subscribe_message(
    client.clone(),
    stream_handler,
    subscribe,
    context.clone(),
    true, // is_switch
  )
  .await
  {
    error!("error handling switch subscribe message: {:?}", e);
    Err(e)
  } else {
    info!("switch subscribe message handled successfully");

    // update the switch context
    client
      .switch_context
      .add_or_update_switch_item(new_full_track_name, SwitchStatus::Next)
      .await;

    let switch_from_track_name = switch_from_track.full_track_name.clone();

    client
      .switch_context
      .add_or_update_switch_item(switch_from_track_name, SwitchStatus::Current)
      .await;

    Ok(())
  }
}

pub async fn handle(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
  opening_request_id: Option<u64>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::Subscribe(m) => {
      handle_subscribe_message(client, stream_handler, *m, context, false).await
    }
    ControlMessage::RequestUpdate(m) => {
      let Some(target_request_id) = opening_request_id else {
        return Err(TerminationCode::ProtocolViolation);
      };
      handle_request_update(client, stream_handler, *m, context, target_request_id).await
    }
    ControlMessage::Switch(m) => handle_switch_message(client, stream_handler, *m, context).await,
    _ => {
      // no-op
      Ok(())
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn upstream_errors_map_to_a_publish_done_status() {
    for (error, status) in [
      (
        RequestErrorCode::Unauthorized,
        PublishDoneStatusCode::Unauthorized,
      ),
      (
        RequestErrorCode::GoingAway,
        PublishDoneStatusCode::GoingAway,
      ),
      (
        RequestErrorCode::MalformedTrack,
        PublishDoneStatusCode::MalformedTrack,
      ),
      (
        RequestErrorCode::ExcessiveLoad,
        PublishDoneStatusCode::ExcessiveLoad,
      ),
      (
        RequestErrorCode::DoesNotExist,
        PublishDoneStatusCode::TrackEnded,
      ),
      // No counterpart: a subscriber can only be told something went wrong.
      (
        RequestErrorCode::InvalidRange,
        PublishDoneStatusCode::InternalError,
      ),
      (
        RequestErrorCode::Timeout,
        PublishDoneStatusCode::InternalError,
      ),
    ] {
      assert_eq!(publish_done_status_for(error), status, "for {error:?}");
    }
  }
}
