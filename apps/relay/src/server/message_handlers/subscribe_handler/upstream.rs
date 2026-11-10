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

//! The upstream half of SUBSCRIBE: the relay's own request to the publisher and its answers.

use crate::server::message_handlers::parameters;
use crate::server::session::client::MOQTClient;
use crate::server::session::session_context::SessionContext;
use crate::server::track::{
  Track, TrackOrigin, TrackStatus, UpstreamRequestCancellation, UpstreamRequestKind,
  await_publisher_streams,
};
use anyhow::Result;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::StreamResetCode;
use moqtail::model::error::TerminationCode;
use moqtail::model::property::track_property::has_unsupported_mandatory;
use moqtail::model::{
  common::reason_phrase::ReasonPhrase, control::control_message::ControlMessage,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Forward a SUBSCRIBE to the upstream publisher on its own bidirectional request
/// stream, then read the response (and follow-ups) on that stream and dispatch
/// them so the result fans out to the downstream subscribers.
pub(crate) async fn forward_subscribe_upstream(
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
    .upstream_requests
    .remove_subscribe(relay_request_id)
    .await;
}

async fn upstream_subscribe_exchange(
  publisher: Arc<MOQTClient>,
  new_sub: Subscribe,
  context: Arc<SessionContext>,
) {
  let relay_request_id = new_sub.request_id;
  let full_track_name = new_sub.get_full_track_name();
  let fail_unanswered = || {
    fail_upstream_subscription(
      relay_request_id,
      publisher.connection_id,
      &full_track_name,
      &context,
    )
  };

  let (send, recv) = match publisher.connection.open_bi().await {
    Ok(streams) => streams,
    Err(e) => {
      error!("Failed to open upstream subscribe stream: {:?}", e);
      fail_unanswered().await;
      return;
    }
  };
  // The response returns on this stream, so correlate it by the relay request id
  // we sent upstream rather than a field in the response.
  let mut upstream = ControlStreamHandler::new(send, recv).with_peer_id(publisher.connection_id);
  if let Err(e) = upstream
    .send(&ControlMessage::Subscribe(Box::new(new_sub)))
    .await
  {
    error!("Failed to send upstream SUBSCRIBE: {:?}", e);
    fail_unanswered().await;
    return;
  }

  let Some(track) = context.track_manager.get_track(&full_track_name).await else {
    upstream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
    return;
  };
  let mut cancel_rx = {
    let _handover = track.lock_handover().await;
    let receiver = track
      .register_upstream_request(
        relay_request_id,
        publisher.connection_id,
        UpstreamRequestKind::Subscribe,
      )
      .await;
    let reason = if publisher.is_track_malformed(&full_track_name).await {
      Some(UpstreamRequestCancellation::MalformedTrack)
    } else if track.subscriber_count().await == 0 {
      Some(UpstreamRequestCancellation::Cancelled)
    } else {
      None
    };
    if let Some(reason) = reason {
      track
        .cancel_upstream_request(relay_request_id, reason)
        .await;
    }
    receiver
  };

  // A publisher owes exactly one SUBSCRIBE_OK or REQUEST_ERROR. One that stays
  // connected and sends neither would otherwise hold the subscriber's request stream
  // open forever, and — where several publishers were asked — keep the others' answers
  // from ever being acted on. The deadline only covers that first response; once it
  // arrives the stream lives as long as the subscription does.
  let answer_deadline = tokio::time::sleep(context.server_config.upstream_subscribe_timeout);
  tokio::pin!(answer_deadline);
  let mut answered = false;
  let mut cancelled = false;

  loop {
    tokio::select! {
      biased;
      reason = &mut cancel_rx => {
        let reason = reason.unwrap_or(UpstreamRequestCancellation::Cancelled);
        info!("Resetting upstream SUBSCRIBE stream ({reason:?})");
        cancelled = true;
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
            if let Err(code) =
              handle_subscribe_ok_message(publisher.clone(), relay_request_id, *m, context.clone())
                .await
            {
              warn!(
                "Unacceptable SubscribeOk from publisher {} for {:?}; closing its session: {:?}",
                publisher.connection_id, full_track_name, code
              );
              // The publisher has answered, so the subscriber is owed a response
              // as surely as if it had declined.
              let err = RequestError::new(
                RequestErrorCode::InternalError,
                0,
                ReasonPhrase::try_new("Upstream subscription failed".to_string()).unwrap(),
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
              publisher
                .connection
                .close(code.to_u32(), b"Unacceptable SUBSCRIBE_OK");
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
            if let Some(track) = context.track_manager.get_track(&full_track_name).await {
              // This publisher is done with the track, which is not the same as
              // the track being over: a PUBLISH_DONE can carry any reason this
              // one publisher stopped. While another still serves the track the
              // subscribers keep receiving it, so they are told nothing and the
              // switching sets are left alone.
              let still_served = track
                .remove_publisher_quietly(publisher.connection_id)
                .await;
              if still_served {
                info!(
                  "Upstream PUBLISH_DONE for {:?} from publisher {}; other publishers still serve it",
                  full_track_name, publisher.connection_id
                );
                return;
              }

              // The message overtakes the data streams it accounts for, so hold it
              // until its Stream Count of them has finished arriving. Passing it on
              // first closes the downstream streams mid-object and understates the
              // Stream Count the relay reports in turn.
              await_publisher_streams(
                &track,
                publisher.connection_id,
                m.stream_count,
                context.server_config.publish_done_stream_timeout,
                context.server_config.publish_done_datagram_quiet,
              )
              .await;

              // SSTS: the track is no longer available upstream; remove it
              // from the switching sets of its subscribers.
              track.remove_from_subscriber_switching_sets().await;
              if let Err(e) = track
                .notify_publish_done(m.status_code, m.reason_phrase.as_str().to_string())
                .await
              {
                error!("Failed to relay upstream PUBLISH_DONE downstream: {:?}", e);
              }
            }
            return;
          }
          Ok(ControlMessage::RequestUpdate(_)) => {
            super::super::close_on_unexpected_request_update(&publisher, "SUBSCRIBE");
            let err = RequestError::new(
              RequestErrorCode::InternalError,
              0,
              ReasonPhrase::try_new("Upstream subscription failed".to_string()).unwrap(),
            );
            // The publisher's session is gone, and the track with it.
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
  if cancelled {
    cleanup_cancelled_upstream_subscription(
      publisher.connection_id,
      &full_track_name,
      answered,
      &context,
    )
    .await;
  }
}

async fn fail_upstream_subscription(
  relay_request_id: u64,
  publisher_connection_id: usize,
  full_track_name: &FullTrackName,
  context: &Arc<SessionContext>,
) {
  let error = RequestError::new(
    RequestErrorCode::InternalError,
    0,
    ReasonPhrase::try_new("Upstream subscription failed".to_string()).unwrap(),
  );
  end_upstream_subscription(
    relay_request_id,
    publisher_connection_id,
    full_track_name,
    error,
    PublishDoneStatusCode::InternalError,
    context.clone(),
  )
  .await;
}

async fn cleanup_cancelled_upstream_subscription(
  publisher_connection_id: usize,
  full_track_name: &FullTrackName,
  answered: bool,
  context: &SessionContext,
) {
  let Some(track) = context.track_manager.get_track(full_track_name).await else {
    return;
  };
  if !answered {
    track.upstream_subscribe_answered();
    return;
  }

  let _ = remove_upstream_publisher(&track, publisher_connection_id, context).await;
}

pub(crate) async fn remove_upstream_publisher(
  track: &Track,
  publisher_connection_id: usize,
  context: &SessionContext,
) -> bool {
  let alias = track
    .publisher_aliases
    .read()
    .await
    .get(&publisher_connection_id)
    .copied();
  let still_served = track
    .remove_publisher_quietly(publisher_connection_id)
    .await;
  if let Some(alias) = alias {
    context
      .track_manager
      .remove_publisher_alias(publisher_connection_id, alias)
      .await;
  }
  still_served
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
      let confirmed = matches!(track.status().await, TrackStatus::Confirmed { .. });
      // This publisher has answered; whatever remains counted has not.
      let remaining = track.upstream_subscribe_answered();
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
  let still_served = { remove_upstream_publisher(&track, publisher_connection_id, &context).await };

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
  {
    // SSTS: the track is no longer available upstream; remove it from the
    // switching sets of its subscribers.
    track.remove_from_subscriber_switching_sets().await;
    if let Err(e) = track
      .notify_publish_done(status_code, error.reason_phrase.as_str().to_string())
      .await
    {
      error!("Failed to end downstream subscriptions: {:?}", e);
    }
  }
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

  // The SUBSCRIBE the relay forwarded, which this answers.
  let Some(sub_request) = context.upstream_requests.get_subscribe(request_id).await else {
    warn!("request id is not verified: {:?}", request_id);
    return Ok(());
  };
  info!("request id is verified: {:?}", request_id);

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

  // The subscription this answers was let go of when its last subscriber left; its
  // upstream stream has been reset.
  if track_arc.origin() == TrackOrigin::CacheOnly {
    info!(
      "SubscribeOk {} answers a subscription {:?} has let go of; ignoring",
      request_id, full_track_name
    );
    return Ok(());
  }

  // Register the publisher's alias for data stream routing.
  if !context
    .track_manager
    .add_track_alias(
      publisher.connection_id,
      msg.track_alias,
      full_track_name.clone(),
    )
    .await
  {
    return Err(TerminationCode::DuplicateTrackAlias);
  }

  // Confirm the track with publisher's metadata
  publisher
    .add_cleanup_track(&track_arc.full_track_name)
    .await;
  let confirmed_now = {
    let track = &track_arc;
    let confirmed_now = track
      .confirm(
        publisher.connection_id,
        msg.track_alias,
        msg.subscribe_parameters.clone(),
        msg.track_properties.clone(),
      )
      .await;
    // This publisher has answered.
    track.upstream_subscribe_answered();
    confirmed_now
  };

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
    let track = &track_arc;
    parameters::downstream_subscribe_ok(&msg.subscribe_parameters, track.largest_object().await)
  };

  // Send SubscribeOk to the FIRST subscriber (the creator)
  {
    let subscriber = { context.client_manager.get(sub_request.requested_by).await };
    if let Some(subscriber) = subscriber {
      let (cached_properties, subscription) = {
        let track = &track_arc;
        (
          track.track_properties().await,
          track
            .get_subscription(&(subscriber.connection_id, sub_request.original_request_id))
            .await,
        )
      };
      // The subscriber can cancel while the publisher is still answering; then
      // there is no one left to tell.
      if let Some(subscription) = subscription {
        let properties = subscription
          .read()
          .await
          .properties_for_ok(cached_properties)
          .await;
        let subscribe_ok = moqtail::model::control::subscribe_ok::SubscribeOk::new(
          subscription.read().await.track_alias,
          downstream_params.clone(),
          properties,
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
        } else {
          subscription.read().await.mark_alias_announced();
          crate::server::subscription::fill::open_fill_fetch_stream(
            context.clone(),
            track_arc.clone(),
            subscription,
            sub_request.original_request_id,
          )
          .await;
        }
      } else {
        info!(
          "creator subscriber request {} cancelled before the publisher answered",
          sub_request.original_request_id
        );
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
    let pending = track_arc.take_pending_subscribers().await;

    for (subscriber_request_id, subscriber_connection_id) in pending {
      let subscriber = { context.client_manager.get(subscriber_connection_id).await };
      if let Some(subscriber) = subscriber {
        let (cached_properties, subscription) = {
          let track = &track_arc;
          (
            track.track_properties().await,
            track
              .get_subscription(&(subscriber.connection_id, subscriber_request_id))
              .await,
          )
        };
        // The subscriber can cancel while the publisher is still answering; then
        // there is no one left to tell.
        let Some(subscription) = subscription else {
          info!(
            "pending subscriber request {} cancelled before the publisher answered",
            subscriber_request_id
          );
          continue;
        };
        let properties = subscription
          .read()
          .await
          .properties_for_ok(cached_properties)
          .await;
        let subscribe_ok = moqtail::model::control::subscribe_ok::SubscribeOk::new(
          subscription.read().await.track_alias,
          downstream_params.clone(),
          properties,
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
        } else {
          subscription.read().await.mark_alias_announced();
          crate::server::subscription::fill::open_fill_fetch_stream(
            context.clone(),
            track_arc.clone(),
            subscription,
            subscriber_request_id,
          )
          .await;
        }
      }
    }
  }

  // Subscription was already added in the Subscribe handler,
  // so we do NOT call add_subscription again here.
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

  // The SUBSCRIBE the relay forwarded, which this answers.
  let Some(sub_request) = context.upstream_requests.remove_subscribe(request_id).await else {
    warn!("RequestError for unknown request id: {:?}", request_id);
    return Ok(());
  };

  let full_track_name = sub_request.original_subscribe_request.get_full_track_name();

  // Mark track as Rejected (if it exists). A track that let go of its upstream
  // subscription before this answer came keeps what it has cached.
  let track_arc = match context.track_manager.get_track(&full_track_name).await {
    Some(track) if track.origin() == TrackOrigin::CacheOnly => {
      info!(
        "RequestError {} answers a subscription {:?} has let go of; ignoring",
        request_id, full_track_name
      );
      return Ok(());
    }
    track => track,
  };
  if let Some(track_arc) = &track_arc {
    let track = &track_arc;
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
    let track = &track_arc;
    let pending = track.take_pending_subscribers().await;

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

  // Nothing upstream serves the track: it stays only if it holds cached Objects.
  if track_arc.is_some() {
    context.track_manager.retire_track(&full_track_name).await;
  }

  Ok(())
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
