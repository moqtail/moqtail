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

use crate::server::message_handlers::parameters;
use crate::server::requests::DownstreamRequest;
use crate::server::session::Session;
use crate::server::session::client::MOQTClient;
use crate::server::session::client::switch_context::SwitchPlan;
use crate::server::session::session_context::SessionContext;
use crate::server::subscription::Subscription;
use crate::server::subscription::filter_constructor;
use crate::server::subscription::subscription_manager::SubscriptionId;
use crate::server::track::{Cleanup, Track, TrackOrigin, TrackStatus};
use anyhow::Result;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::model::{
  common::reason_phrase::ReasonPhrase, control::control_message::ControlMessage,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::SubscribeRequest;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, info, trace, warn};

mod request_update;
pub(crate) mod upstream;

use request_update::handle_request_update;
use upstream::forward_subscribe_upstream;

async fn add_subscription(
  subscribe: Subscribe,
  track: &Track,
  subscriber: Arc<MOQTClient>,
) -> Option<Arc<RwLock<Subscription>>> {
  let request_id = subscribe.request_id;
  match track.add_subscription(subscriber.clone(), subscribe).await {
    Ok(subscription) => {
      subscriber
        .subscriptions
        .add(request_id, Arc::downgrade(&subscription))
        .await;
      Some(subscription)
    }
    // Only a Request ID already in use on this track is refused, and the subscription
    // manager has logged it.
    Err(_) => None,
  }
}

/// Drop a subscription that must be rejected after `add_subscription` already
/// registered it, on the track and on the client.
async fn reject_subscription(
  track_arc: &Arc<Track>,
  client: &MOQTClient,
  subscription_id: &SubscriptionId,
) {
  {
    let track = &track_arc;
    track.remove_subscription(subscription_id).await;
  }
  client.subscriptions.remove(subscription_id.1).await;
}

/// Reads and checks a SWITCH_FROM carried by a SUBSCRIBE or REQUEST_UPDATE.
///
/// `Ok(None)` when there is no SWITCH_FROM and this is an ordinary request.
/// `Err` carries the REQUEST_ERROR to answer with: the switch cannot be
/// performed, which is not grounds for closing the session.
pub(crate) async fn plan_switch(
  client: &Arc<MOQTClient>,
  context: &Arc<SessionContext>,
  activating_request_id: u64,
  activating: &FullTrackName,
  parameters: &[MessageParameter],
) -> Result<Option<SwitchPlan>, RequestError> {
  let Some(MessageParameter::SwitchFrom {
    request_id,
    mode,
    publish_done,
  }) = parameters
    .iter()
    .find(|p| matches!(p, MessageParameter::SwitchFrom { .. }))
    .cloned()
  else {
    return Ok(None);
  };

  let invalid = |reason: &str| {
    RequestError::new(
      RequestErrorCode::InvalidSwitch,
      0,
      ReasonPhrase::try_new(reason.to_string()).unwrap(),
    )
  };

  // The switch decides the activating subscription's Forward State itself.
  if parameters
    .iter()
    .any(|p| matches!(p, MessageParameter::Forward { .. }))
  {
    return Err(invalid("SWITCH_FROM cannot be combined with FORWARD"));
  }

  if request_id == activating_request_id {
    return Err(invalid("a subscription cannot switch away from itself"));
  }

  let Some((_, subscription)) = context
    .track_manager
    .find_subscription_by_request_id(client.connection_id, request_id)
    .await
  else {
    return Err(invalid("no such subscription to switch away from"));
  };

  // A suspending subscription in Forward State 0 is not an error; there is simply
  // less to stop.
  let suspending = subscription.read().await.full_track_name.clone();

  // Both subscriptions would deliver the same Objects, so there is no switch to make.
  if suspending == *activating {
    return Err(invalid(
      "a subscription cannot switch away from its own track",
    ));
  }

  Ok(Some(SwitchPlan::new(
    suspending,
    request_id,
    mode,
    publish_done,
  )))
}

/// The subscriber's RENDEZVOUS_TIMEOUT capped at `max`; zero when absent.
fn rendezvous_timeout(parameters: &[MessageParameter], max: Duration) -> Duration {
  parameters
    .iter()
    .find_map(|p| match p {
      MessageParameter::RendezvousTimeout { timeout } => Some(Duration::from_millis(*timeout)),
      _ => None,
    })
    .map_or(Duration::ZERO, |requested| requested.min(max))
}

async fn handle_subscribe_message(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  sub: Subscribe,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  info!("received Subscribe message: {:?}", sub);
  let track_namespace = sub.track_namespace.clone();
  let full_track_name = sub.get_full_track_name();

  // The relay applies Range Filters itself, so ones it cannot honour are refused
  // before anything is created rather than dropped.
  if let Err(e) = filter_constructor::object_filter(
    &sub.subscribe_parameters,
    context.server_config.max_filter_ranges,
  ) {
    info!("Rejecting SUBSCRIBE: {e}");
    stream_handler
      .send_impl(&filter_constructor::invalid_filter(&e))
      .await
      .unwrap();
    return Ok(());
  }

  // A SUBSCRIBE carrying SWITCH_FROM activates this subscription and suspends
  // another. Checked first: a switch that cannot be performed is refused without
  // creating anything.
  let switch_plan = match plan_switch(
    &client,
    &context,
    sub.request_id,
    &full_track_name,
    &sub.subscribe_parameters,
  )
  .await
  {
    Ok(plan) => plan,
    Err(err) => {
      info!(
        "Rejecting SUBSCRIBE with SWITCH_FROM: {}",
        err.reason_phrase.as_str()
      );
      stream_handler.send_impl(&err).await.unwrap();
      return Ok(());
    }
  };
  // A switch applies its filter and Forward State in begin_switch, well after the
  // SUBSCRIBE_OK goes out; anything measured against the subscription's filter has to
  // wait for that.
  let switching = switch_plan.is_some();

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

  let rendezvous = rendezvous_timeout(
    &sub.subscribe_parameters,
    context.server_config.max_rendezvous_timeout,
  );

  // Every publisher of the exact Track, plus every publisher that announced a namespace
  // it falls under. A SUBSCRIBE goes to all of them, not to whichever matched first.
  let publishers = {
    trace!("trying to get the publishers");
    context
      .client_manager
      .wait_for_publishers(&full_track_name, rendezvous)
      .await
  };

  if publishers.is_empty() {
    info!(
      "no publisher found for track namespace: {:?}",
      track_namespace
    );
    // TIMEOUT only when the subscriber asked to wait.
    let (code, reason) = if rendezvous.is_zero() {
      (RequestErrorCode::DoesNotExist, "Unknown track namespace")
    } else {
      (RequestErrorCode::Timeout, "No publisher appeared in time")
    };
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
      code,
      0, //TODO: Maybe decide on another retry interval?
      ReasonPhrase::try_new(reason.to_string()).unwrap(),
    );
    // The subscriber may have given up while it waited.
    if let Err(e) = stream_handler.send_impl(&subscribe_error).await {
      info!("Could not answer an unmatched SUBSCRIBE: {:?}", e);
    }
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

  // Scoped so the track guard is gone before anything below reaches for it again.
  // A cache-only track has no upstream subscription, so the SUBSCRIBE that takes it
  // over forwards upstream as if it had created it. Taken over and joined under one
  // lock, so the track cannot let go of its upstream subscription in between.
  let (is_creator, subscription) = {
    let track = &track_arc;
    let _handover = track.lock_handover().await;
    let taken_over = track
      .take_over(TrackOrigin::Subscribe, TrackStatus::Pending)
      .await;
    (
      is_creator || taken_over,
      add_subscription(sub.clone(), track, client.clone()).await,
    )
  };
  let Some(subscription) = subscription else {
    warn!(
      "Rejecting SUBSCRIBE from {} for {:?}: request {} already has a subscription",
      context.connection_id, &full_track_name, sub.request_id
    );
    let err = RequestError::new(
      RequestErrorCode::InternalError,
      0,
      ReasonPhrase::try_new("could not add the subscription".to_string()).unwrap(),
    );
    stream_handler.send_impl(&err).await.unwrap();
    return Ok(());
  };

  // Sender-side track switching: register this track in its switching set.
  if let Some(MessageParameter::SwitchingSetAssignment {
    switching_set_id,
    algorithm_id,
    throughput_threshold_kbps,
    set_throughput_weight,
    activate_switching,
    set_rank,
  }) = sub
    .subscribe_parameters
    .iter()
    .find(|p| matches!(p, MessageParameter::SwitchingSetAssignment { .. }))
  {
    // Read before taking the set manager's lock: removing a track takes the
    // other way around (subscription -> set manager), so holding one lock
    // while grabbing the other would deadlock.
    let relay_track_id = track_arc.relay_track_id;

    if let Err(e) = client.ssts.validate_assignment(*algorithm_id) {
      warn!("Rejecting SUBSCRIBE from {}: {}", context.connection_id, e);
      reject_subscription(&track_arc, &client, &(client.connection_id, sub.request_id)).await;
      let err = RequestError::new(
        RequestErrorCode::UnsupportedExtension,
        0,
        ReasonPhrase::try_new(e).unwrap(),
      );
      stream_handler.send_impl(&err).await.unwrap();
      return Ok(());
    }

    {
      let mut manager = client.ssts.switching_sets.write().await;
      if let Err(e) = manager.assign(
        full_track_name.clone(),
        relay_track_id,
        sub.request_id,
        *switching_set_id,
        *algorithm_id,
        *throughput_threshold_kbps,
        *set_throughput_weight,
        *activate_switching,
        *set_rank,
      ) {
        warn!("Rejecting SUBSCRIBE from {}: {}", context.connection_id, e);
        // A track MUST only be assigned to one switching set at a time; the
        // subscription is rejected. There is no dedicated parameter-error
        // code to return, so the unsupported-extension one carries it.
        drop(manager);
        reject_subscription(&track_arc, &client, &(client.connection_id, sub.request_id)).await;
        let err = RequestError::new(
          RequestErrorCode::UnsupportedExtension,
          0,
          ReasonPhrase::try_new(e.to_string()).unwrap(),
        );
        stream_handler.send_impl(&err).await.unwrap();
        return Ok(());
      }
    }

    // The spec starts switching-set subscriptions with Forward=0; keep this
    // subscription forwarding so the Object gating can select per group.
    let sub = subscription.read().await;
    let mut state = sub.subscription_state.write().await;
    state.forward = true;
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
      let track = &track_arc;
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
      context
        .upstream_requests
        .add_subscribe(new_sub.request_id, req.clone())
        .await;
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
    let track = &track_arc;
    let status = track
      .join_status(sub.request_id, context.connection_id)
      .await;

    match status {
      TrackStatus::Confirmed {
        upstream_parameters,
      } => {
        info!(
          "Track confirmed, sending SubscribeOk to subscriber {}",
          client.connection_id
        );
        let cached_properties = track.track_properties().await;
        let params =
          parameters::downstream_subscribe_ok(&upstream_parameters, track.largest_object().await);
        let properties = subscription
          .read()
          .await
          .properties_for_ok(cached_properties)
          .await;
        let subscribe_ok = moqtail::model::control::subscribe_ok::SubscribeOk::new(
          subscription.read().await.track_alias,
          params,
          properties,
        );
        let sent = stream_handler.send_impl(&subscribe_ok).await;
        if sent.is_ok() {
          subscription.read().await.mark_alias_announced();
          // A switch decides the filter this fill is measured against, and it has not
          // been applied yet: the fill for one is opened once begin_switch has run.
          if !switching {
            crate::server::subscription::fill::open_fill_fetch_stream(
              context.clone(),
              track_arc.clone(),
              subscription.clone(),
              sub.request_id,
            )
            .await;
          }
        }
        sent
      }
      TrackStatus::Pending => {
        info!(
          "Track pending, subscriber {} will wait for confirmation",
          client.connection_id
        );
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

  if res.is_ok()
    && let Some(plan) = switch_plan
  {
    // Scoped: begin_switch reaches for locks of its own, and the track guard must
    // be gone before it does.
    let largest = track_arc.largest_object().await;
    {
      // A SUBSCRIBE describes the subscription in full, so a filter it leaves out
      // is no filter.
      let filter = crate::server::subscription::subscription_state::location_filter_of(
        &sub.subscribe_parameters,
      );
      subscription
        .read()
        .await
        .begin_switch(plan, Some(&filter), largest)
        .await;

      // Now that the switch has said where this subscription starts and that it is
      // forwarding, the fill can be measured: everything already published from the
      // group it joins, so the subscriber has the group from its start rather than
      // from wherever it happens to arrive.
      crate::server::subscription::fill::open_fill_fetch_stream(
        context.clone(),
        track_arc.clone(),
        subscription,
        sub.request_id,
      )
      .await;
    }
  }

  // Store in client's subscribe requests on success
  if res.is_ok() {
    let orig_req = SubscribeRequest::new(original_request_id, context.connection_id, sub, None);
    client
      .downstream_requests
      .insert(
        original_request_id,
        DownstreamRequest::Subscribe(orig_req.clone()),
      )
      .await;

    trace!(
      "inserted request into client's subscribe requests: {:?}",
      orig_req
    );
  } else {
    error!("error in adding subscription: {:?}", res);
  }
  res
}

/// Cancel a subscription when its SUBSCRIBE request stream is reset or closed.
pub(crate) async fn cancel_subscription(
  client: Arc<MOQTClient>,
  request_id: u64,
  context: &Arc<SessionContext>,
) {
  // find the track alias by using the request id
  let full_track_name = {
    let Some(request) = client.downstream_requests.get_subscribe(request_id).await else {
      warn!("request not found for request id: {:?}", request_id);
      return;
    };
    request.original_subscribe_request.get_full_track_name()
  };

  // Sender-side track switching: this subscription no longer holds the track in
  // its switching set. Another subscription to the same track may still.
  {
    let mut manager = client.ssts.switching_sets.write().await;
    manager.release(&full_track_name, request_id);
  }

  // remove the subscription from the track
  let track_option = context.track_manager.get_track(&full_track_name).await;

  if let Some(track_lock) = track_option {
    // The subscriber may hold other subscriptions to this track under other request
    // ids. Cancel only the one this request owns, and only if it is still there.
    let owned = track_lock
      .has_subscription(&(context.connection_id, request_id))
      .await;

    if !owned {
      info!(
        "Subscription cancel: request {} no longer owns the subscription for {:?}; leaving it",
        request_id, full_track_name
      );
      client.downstream_requests.remove(request_id).await;
      client.subscriptions.remove(request_id).await;
      return;
    }

    // A SUBSCRIBE-created track whose last subscriber left has its upstream
    // subscription cancelled. It becomes cache-only if it holds cached Objects, and is
    // removed otherwise; either way the next SUBSCRIBE subscribes upstream afresh
    // rather than being answered from it. A PUBLISH-created track has a publisher still
    // pushing to it and outlives any number of subscribers.
    let cleanup = track_lock
      .remove_subscriber(&(context.connection_id, request_id))
      .await;
    if cleanup != Cleanup::Nothing {
      info!(
        "Last subscriber left {:?}: {:?}; next SUBSCRIBE will re-subscribe upstream",
        full_track_name, cleanup
      );
      context
        .track_manager
        .clean_up(&full_track_name, cleanup)
        .await;
    }
  } else {
    warn!(
      "Subscription cancel: Track {:?} already removed.",
      full_track_name
    );
  }

  // remove the subscription from the client
  client.subscriptions.remove(request_id).await;

  // Remove the request from the client's request map so it doesn't leak
  {
    client.downstream_requests.remove(request_id).await;

    trace!(
      "Cleaned up client subscribe request {} on cancel",
      request_id
    );
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
      handle_subscribe_message(client, stream_handler, *m, context).await
    }
    ControlMessage::RequestUpdate(m) => {
      let Some(target_request_id) = opening_request_id else {
        return Err(TerminationCode::ProtocolViolation);
      };
      handle_request_update(client, stream_handler, *m, context, target_request_id).await
    }
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
  fn rendezvous_timeout_defaults_to_zero_and_is_capped() {
    let max = Duration::from_millis(1000);
    assert_eq!(rendezvous_timeout(&[], max), Duration::ZERO);
    assert_eq!(
      rendezvous_timeout(&[MessageParameter::new_rendezvous_timeout(500)], max),
      Duration::from_millis(500)
    );
    assert_eq!(
      rendezvous_timeout(&[MessageParameter::new_rendezvous_timeout(5000)], max),
      max
    );
    assert_eq!(
      rendezvous_timeout(
        &[MessageParameter::new_rendezvous_timeout(500)],
        Duration::ZERO
      ),
      Duration::ZERO
    );
  }
}
