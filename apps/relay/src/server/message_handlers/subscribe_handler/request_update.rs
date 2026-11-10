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

//! REQUEST_UPDATE for a subscription this relay serves.

use crate::server::requests::DownstreamRequest;
use crate::server::session::client::MOQTClient;
use crate::server::session::session_context::SessionContext;
use crate::server::subscription::filter_constructor;
use anyhow::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::request_ok::RequestOk;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::error::TerminationCode;
use moqtail::model::parameter::message_parameter::{
  MessageParameter, apply_message_parameter_update,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{error, info, warn};

use super::plan_switch;

pub async fn handle_request_update(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  update_msg: moqtail::model::control::request_update::RequestUpdate,
  context: Arc<SessionContext>,
  existing_req_id: u64,
) -> Result<(), TerminationCode> {
  // An update can carry SWITCH_FROM too, activating the subscription it arrives on.
  let asks_for_a_fill = update_msg
    .parameters
    .iter()
    .any(|p| matches!(p, MessageParameter::FillParameters { .. }));
  // The update itself is handed on to the subscription, so a switch's filter is
  // kept here rather than read back off it.
  let update_parameters = update_msg.parameters.clone();

  // Read before anything is applied: a refused switch must leave the subscription
  // as it was.
  let (full_track_name, current_parameters) = {
    match client
      .downstream_requests
      .get_subscribe(existing_req_id)
      .await
    {
      Some(req) => (
        req.original_subscribe_request.get_full_track_name(),
        req.original_subscribe_request.subscribe_parameters.clone(),
      ),
      None => {
        warn!(
          "RequestUpdate existing_request_id {} is not a valid Subscribe request for this client",
          existing_req_id
        );
        return Err(TerminationCode::ProtocolViolation);
      }
    }
  };

  // Filters the subscription would be left with but that cannot be honoured fail
  // the update before any of it is applied.
  if let Err(e) = filter_constructor::updated_filter(
    &current_parameters,
    &update_msg.parameters,
    context.server_config.max_filter_ranges,
  ) {
    info!("Refusing REQUEST_UPDATE for {:?}: {e}", full_track_name);
    fail_update(
      &client,
      stream_handler,
      &context,
      &full_track_name,
      existing_req_id,
      filter_constructor::invalid_filter(&e),
    )
    .await;
    return Ok(());
  }

  let switch_plan = match plan_switch(
    &client,
    &context,
    existing_req_id,
    &full_track_name,
    &update_msg.parameters,
  )
  .await
  {
    Ok(plan) => plan,
    Err(err) => {
      info!(
        "Rejecting REQUEST_UPDATE with SWITCH_FROM: {}",
        err.reason_phrase.as_str()
      );
      let _ = stream_handler.send_impl(&err).await;
      return Ok(());
    }
  };

  client
    .downstream_requests
    .update(existing_req_id, |request| match request {
      DownstreamRequest::Subscribe(req) => {
        apply_message_parameter_update(
          &mut req.original_subscribe_request.subscribe_parameters,
          update_msg.parameters.clone(),
        );
        Some(())
      }
      _ => None,
    })
    .await;

  // Sender-side track switching: apply switching set parameter updates. All
  // fields are present in the parameter, and the most recently received message
  // wins. Applied with the rest of the update, after the switch plan was
  // accepted, so a rejected update leaves the switching set as it was.
  if let Some(p) = update_msg
    .parameters
    .iter()
    .find(|p| matches!(p, MessageParameter::SwitchingSetAssignment { .. }))
    && let MessageParameter::SwitchingSetAssignment {
      algorithm_id,
      throughput_threshold_kbps,
      set_throughput_weight,
      activate_switching,
      set_rank,
      ..
    } = p
  {
    let mut manager = client.ssts.switching_sets.write().await;
    match manager.update_assignment(
      &full_track_name,
      Some(*algorithm_id),
      Some(*throughput_threshold_kbps),
      Some(*set_throughput_weight),
      Some(*activate_switching),
      Some(*set_rank),
    ) {
      Ok(warnings) => {
        for warning in warnings {
          warn!("SSTS REQUEST_UPDATE for {full_track_name}: {warning}");
        }
      }
      // The rest of the update still applies; only the switching set part could
      // not be, and it is the track that is not in a set.
      Err(e) => warn!("SSTS REQUEST_UPDATE for {full_track_name}: {e}"),
    }
  }

  // 2. Get the track instance
  let track_lock = context.track_manager.get_track(&full_track_name).await;

  if track_lock.is_none() {
    warn!("Track not found for track name: {:?}", full_track_name);
    return Err(TerminationCode::ProtocolViolation);
  }

  let track_arc = track_lock.unwrap();

  // The subscription's lock is released before responding.
  let update_result = {
    match track_arc
      .get_subscription(&(client.connection_id, existing_req_id))
      .await
    {
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
      super::super::publish_handler::ensure_upstream_forwarding(&track_arc, &context).await;
      let ok_msg = RequestOk::new(vec![]);
      let _ = stream_handler.send_impl(&ok_msg).await;

      // Scoped: what follows reaches for locks of its own, and the track guard
      // must be gone before it does.
      let (subscription, largest) = {
        let track = &track_arc;
        (
          track
            .get_subscription(&(client.connection_id, existing_req_id))
            .await,
          track.largest_object().await,
        )
      };

      // An update changes only what it carries, so a filter it leaves out leaves
      // the subscription's own in place.
      let filter = update_parameters.iter().find_map(|p| match p {
        MessageParameter::LocationFilter { filter } => Some(filter),
        _ => None,
      });

      if let Some(subscription) = &subscription {
        if let Some(plan) = switch_plan {
          subscription
            .read()
            .await
            .begin_switch(plan, filter, largest)
            .await;
        } else if filter.is_some() {
          // A filter the update changed starts where it now says, which for a live
          // one is against the largest Object as of this update.
          subscription.read().await.resolve_live_start(largest).await;
        }
      }

      // An update that carries FILL_PARAMETERS asks for another fill, over
      // whatever range it now specifies.
      if asks_for_a_fill
        && let Some(subscription) = track_arc
          .get_subscription(&(client.connection_id, existing_req_id))
          .await
      {
        crate::server::subscription::fill::open_fill_fetch_stream(
          context.clone(),
          track_arc.clone(),
          subscription,
          existing_req_id,
        )
        .await;
      }
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
      fail_update(
        &client,
        stream_handler,
        &context,
        &full_track_name,
        existing_req_id,
        err_msg,
      )
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
/// Refuses a REQUEST_UPDATE with `error`, then ends the subscription it was for with
/// PUBLISH_DONE(UPDATE_FAILED), which is what a failed update does to a subscription.
async fn fail_update(
  client: &Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  context: &Arc<SessionContext>,
  full_track_name: &FullTrackName,
  request_id: u64,
  error: RequestError,
) {
  let _ = stream_handler.send_impl(&error).await;

  let track_arc = context.track_manager.get_track(full_track_name).await;
  let stream_count = match &track_arc {
    Some(track) => match track
      .get_subscription(&(client.connection_id, request_id))
      .await
    {
      Some(sub) => sub.read().await.opened_stream_count(),
      None => 0,
    },
    None => 0,
  };
  let done = PublishDone::new(
    PublishDoneStatusCode::UpdateFailed,
    stream_count,
    ReasonPhrase::try_new("REQUEST_UPDATE failed".to_string()).unwrap(),
  );
  let _ = stream_handler.send_impl(&done).await;

  if let Some(track) = track_arc {
    track
      .remove_subscription(&(client.connection_id, request_id))
      .await;
  }
  client.subscriptions.remove(request_id).await;
}
