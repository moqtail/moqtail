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
use crate::server::session::Session;
use crate::server::session_context::SessionContext;
use crate::server::track::{Track, TrackStatus};
use core::result::Result;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::error::TerminationCode;
use moqtail::model::{
  common::reason_phrase::ReasonPhrase, control::control_message::ControlMessage,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::SubscribeRequest;
use std::sync::Arc;
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

async fn handle_subscribe_message(
  client: Arc<MOQTClient>,
  control_stream_handler: &mut ControlStreamHandler,
  sub: Subscribe,
  context: Arc<SessionContext>,
  is_switch: bool,
) -> Result<(), TerminationCode> {
  info!("received Subscribe message: {:?}", sub);
  let track_namespace = sub.track_namespace.clone();
  let request_id = sub.request_id;
  let full_track_name = sub.get_full_track_name();

  // check request id
  {
    let max_request_id = context
      .max_request_id
      .load(std::sync::atomic::Ordering::Relaxed);
    if request_id >= max_request_id {
      warn!(
        "request id ({}) is greater than max request id ({})",
        request_id, max_request_id
      );
      return Err(TerminationCode::TooManyRequests);
    }
  }

  // find who is the publisher
  // first we try with the full track name
  // if not found, we try with the announced track namespace
  // in both cases, the first publisher that satisfies the condition is returned
  // TODO: support multiple publishers
  let publisher = {
    debug!("trying to get the publisher");
    let m = context.client_manager.read().await;
    debug!(
      "client manager obtained, current client id: {}",
      context.connection_id
    );
    match m.get_publisher_by_full_track_name(&full_track_name).await {
      Some(p) => Some(p),
      None => {
        info!(
          "no publisher found for full track name: {:?}",
          &full_track_name
        );
        let m = context.client_manager.read().await;
        debug!(
          "client manager obtained, current client id: {}",
          context.connection_id
        );
        m.get_publisher_by_announced_track_namespace(&track_namespace)
          .await
      }
    }
  };

  let publisher = if let Some(publisher) = publisher {
    publisher.clone()
  } else {
    info!(
      "no publisher found for track namespace: {:?}",
      track_namespace
    );
    // send SubscribeError
    let subscribe_error = moqtail::model::control::subscribe_error::SubscribeError::new(
      sub.request_id,
      moqtail::model::control::constant::SubscribeErrorCode::TrackDoesNotExist,
      ReasonPhrase::try_new("Unknown track namespace".to_string()).unwrap(),
    );
    control_stream_handler
      .send_impl(&subscribe_error)
      .await
      .unwrap();
    return Ok(());
  };

  publisher.add_subscriber(context.connection_id).await;

  info!(
    "Subscriber ({}) added to the publisher ({})",
    context.connection_id, publisher.connection_id
  );

  let original_request_id = sub.request_id;

  // Atomic get-or-create: first subscriber creates, subsequent ones find existing
  let (track_arc, is_creator) = context
    .track_manager
    .get_or_create_track(&full_track_name, || {
      Track::new(
        0, // provisional alias, updated on SubscribeOk from publisher
        full_track_name.clone(),
        publisher.connection_id,
        context.server_config,
        TrackStatus::Pending,
      )
    })
    .await;

  let track = track_arc.read().await;

  add_subscription(sub.clone(), &track, client.clone(), is_switch).await;

  let res: Result<(), TerminationCode> = if is_creator {
    // First subscriber for this track: forward Subscribe to publisher
    info!(
      "First subscriber for track {:?}, forwarding to publisher",
      &full_track_name
    );

    let mut new_sub = sub.clone();
    new_sub.forward = true;
    new_sub.request_id =
      Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

    publisher
      .queue_message(ControlMessage::Subscribe(Box::new(new_sub.clone())))
      .await;

    // Store relay subscribe request mapping
    // TODO: we need to add a timeout here or another loop to control expired requests
    let req = SubscribeRequest::new(
      original_request_id,
      context.connection_id,
      sub.clone(),
      Some(new_sub.clone()),
    );
    let mut requests = context.relay_subscribe_requests.write().await;
    requests.insert(new_sub.request_id, req.clone());
    info!(
      "inserted request into relay's subscribe requests: {:?} with relay's request id: {:?}",
      req, new_sub.request_id
    );
    // Do NOT send SubscribeOk yet -- wait for publisher confirmation
    Ok(())
  } else {
    // Subsequent subscriber: track already exists
    let track = track_arc.read().await;
    let status = track.get_status().await;

    match status {
      TrackStatus::Confirmed {
        publisher_track_alias,
        expires,
        largest_location,
      } => {
        info!(
          "Track confirmed, sending SubscribeOk to subscriber {}",
          client.connection_id
        );
        let subscribe_ok =
          moqtail::model::control::subscribe_ok::SubscribeOk::new_ascending_with_content(
            sub.request_id,
            publisher_track_alias,
            expires,
            largest_location,
            None,
          );
        control_stream_handler.send_impl(&subscribe_ok).await
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
          "Track rejected, sending SubscribeError to subscriber {}",
          client.connection_id
        );
        let subscribe_error = moqtail::model::control::subscribe_error::SubscribeError::new(
          sub.request_id,
          error_code,
          reason_phrase,
        );
        control_stream_handler.send_impl(&subscribe_error).await
      }
    }
  };

  // Store in client's subscribe requests on success
  if res.is_ok() {
    let mut requests = client.subscribe_requests.write().await;
    let orig_req = SubscribeRequest::new(original_request_id, context.connection_id, sub, None);
    requests.insert(original_request_id, orig_req.clone());
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
  _client: Arc<MOQTClient>,
  _control_stream_handler: &mut ControlStreamHandler,
  msg: moqtail::model::control::subscribe_ok::SubscribeOk,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  info!("received SubscribeOk message: {:?}", msg);
  let request_id = msg.request_id;

  // Look up and remove the relay subscribe request (no longer needed after processing)
  let sub_request = {
    let mut requests = context.relay_subscribe_requests.write().await;
    debug!("current requests: {:?}", requests);
    match requests.remove(&request_id) {
      Some(m) => {
        info!("request id is verified: {:?}", request_id);
        m
      }
      None => {
        warn!("request id is not verified: {:?}", request_id);
        return Ok(());
      }
    }
  };

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

  // Confirm the track with publisher's metadata
  {
    let mut track = track_arc.write().await;
    track
      .confirm(msg.track_alias, msg.expires, msg.largest_location.clone())
      .await;
  }

  // Register the track alias for data stream routing
  context
    .track_manager
    .add_track_alias(msg.track_alias, full_track_name.clone())
    .await;

  // Send SubscribeOk to the FIRST subscriber (the creator)
  {
    let subscriber = {
      let mngr = context.client_manager.read().await;
      mngr.get(sub_request.requested_by).await
    };
    if let Some(subscriber) = subscriber {
      let subscribe_ok =
        moqtail::model::control::subscribe_ok::SubscribeOk::new_ascending_with_content(
          sub_request.original_request_id,
          msg.track_alias,
          msg.expires,
          msg.largest_location.clone(),
          None,
        );
      info!(
        "sending SubscribeOk to creator subscriber: {:?}",
        subscriber.connection_id
      );
      subscriber
        .queue_message(ControlMessage::SubscribeOk(Box::new(subscribe_ok)))
        .await;
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
      let subscriber = {
        let mngr = context.client_manager.read().await;
        mngr.get(subscriber_connection_id).await
      };
      if let Some(subscriber) = subscriber {
        let subscribe_ok =
          moqtail::model::control::subscribe_ok::SubscribeOk::new_ascending_with_content(
            subscriber_request_id,
            msg.track_alias,
            msg.expires,
            msg.largest_location.clone(),
            None,
          );
        info!(
          "sending SubscribeOk to pending subscriber: {:?}",
          subscriber.connection_id
        );
        subscriber
          .queue_message(ControlMessage::SubscribeOk(Box::new(subscribe_ok)))
          .await;
      }
    }
  }

  // Subscription was already added in the Subscribe handler,
  // so we do NOT call add_subscription again here.
  Ok(())
}

async fn handle_unsubscribe_message(
  client: Arc<MOQTClient>,
  _control_stream_handler: &mut ControlStreamHandler,
  unsubscribe_message: moqtail::model::control::unsubscribe::Unsubscribe,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  info!("received Unsubscribe message: {:?}", unsubscribe_message);
  // stop sending objects for the track for the subscriber
  // by removing the subscription
  // find the track alias by using the request id
  let requests = client.subscribe_requests.read().await;
  let request = requests.get(&unsubscribe_message.request_id);
  if request.is_none() {
    // a warning is enough
    warn!(
      "request not found for request id: {:?}",
      unsubscribe_message.request_id
    );
    return Ok(());
  }
  let request = request.unwrap();
  let full_track_name = request.original_subscribe_request.get_full_track_name();

  // remove the subscription from the track
  let track_lock = context
    .track_manager
    .get_track(&full_track_name)
    .await
    .unwrap();
  let track = track_lock.write().await;
  track.remove_subscription(context.connection_id).await;

  // remove the subscription from the client
  client
    .subscriptions
    .remove_subscription(&full_track_name)
    .await;

  Ok(())
}

async fn handle_subscribe_update_message(
  client: Arc<MOQTClient>,
  _control_stream_handler: &mut ControlStreamHandler,
  subscribe_update_message: moqtail::model::control::subscribe_update::SubscribeUpdate,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  info!(
    "received SubscribeUpdate message: {:?}",
    subscribe_update_message
  );

  // a subscribe update message contains subscription_request_id
  // which is the request id of the subscription we want to update
  let sub_request_id = subscribe_update_message.subscription_request_id;

  let requests = client.subscribe_requests.read().await;
  let request = requests.get(&sub_request_id);
  if request.is_none() {
    warn!(
      "request not found for subscriber request id: {:?}",
      sub_request_id
    );
    return Err(TerminationCode::ProtocolViolation);
  }
  let request = request.unwrap();

  // we can not get the full track name and hence, the track instance
  let full_track_name = request.original_subscribe_request.get_full_track_name();
  let track_lock = context.track_manager.get_track(&full_track_name).await;

  if track_lock.is_none() {
    warn!("track not found for track name: {:?}", full_track_name);
    return Err(TerminationCode::ProtocolViolation);
  }

  let track_arc = track_lock.unwrap();
  let track_guard = track_arc.read().await;

  if let Some(subscription) = track_guard.get_subscription(context.connection_id).await {
    let sub = subscription.read().await;
    match sub.update_subscription(subscribe_update_message).await {
      Ok(_) => info!(
        "subscription updated, track: {:?} subscriber: {}",
        full_track_name, context.connection_id
      ),
      Err(e) => error!(
        "subscription could not be updated, track: {:?} subscriber: {} error: {:?}",
        full_track_name, context.connection_id, e
      ),
    }
  }
  Ok(())
}

async fn handle_subscribe_error_message(
  _client: Arc<MOQTClient>,
  _control_stream_handler: &mut ControlStreamHandler,
  subscribe_error_message: moqtail::model::control::subscribe_error::SubscribeError,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  info!(
    "received SubscribeError message: {:?}",
    subscribe_error_message
  );
  let msg = subscribe_error_message;
  let request_id = msg.request_id;

  // Look up and remove the relay subscribe request
  let sub_request = {
    let mut requests = context.relay_subscribe_requests.write().await;
    match requests.remove(&request_id) {
      Some(m) => m,
      None => {
        warn!("SubscribeError for unknown request id: {:?}", request_id);
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

  // Send SubscribeError to the FIRST subscriber (the creator)
  {
    let subscriber = {
      let mngr = context.client_manager.read().await;
      mngr.get(sub_request.requested_by).await
    };
    if let Some(subscriber) = subscriber {
      let subscribe_error = moqtail::model::control::subscribe_error::SubscribeError::new(
        sub_request.original_request_id,
        msg.error_code,
        msg.reason_phrase.clone(),
      );
      subscriber
        .queue_message(ControlMessage::SubscribeError(Box::new(subscribe_error)))
        .await;
    }
  }

  // Send SubscribeError to ALL pending subscribers
  if let Some(track_arc) = &track_arc {
    let track = track_arc.read().await;
    let pending = {
      let mut pending = track.pending_subscribers.write().await;
      std::mem::take(&mut *pending)
    };

    for (subscriber_request_id, subscriber_connection_id) in pending {
      let subscriber = {
        let mngr = context.client_manager.read().await;
        mngr.get(subscriber_connection_id).await
      };
      if let Some(subscriber) = subscriber {
        let subscribe_error = moqtail::model::control::subscribe_error::SubscribeError::new(
          subscriber_request_id,
          msg.error_code,
          msg.reason_phrase.clone(),
        );
        subscriber
          .queue_message(ControlMessage::SubscribeError(Box::new(subscribe_error)))
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
  control_stream_handler: &mut ControlStreamHandler,
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

  let subscribe = Subscribe::new_latest_object(
    switch_message.request_id,
    switch_message.track_namespace.clone(),
    switch_message.track_name.clone(),
    0,
    GroupOrder::Original,
    true,
    switch_message.subscribe_parameters.clone(),
  );

  let new_full_track_name = subscribe.get_full_track_name();

  if let Err(e) = handle_subscribe_message(
    client.clone(),
    control_stream_handler,
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
  control_stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::Subscribe(m) => {
      handle_subscribe_message(client, control_stream_handler, *m, context, false).await
    }
    ControlMessage::SubscribeOk(m) => {
      handle_subscribe_ok_message(client, control_stream_handler, *m, context).await
    }
    ControlMessage::Unsubscribe(m) => {
      handle_unsubscribe_message(client, control_stream_handler, *m, context).await
    }
    ControlMessage::SubscribeUpdate(m) => {
      handle_subscribe_update_message(client, control_stream_handler, *m, context).await
    }
    ControlMessage::SubscribeError(m) => {
      handle_subscribe_error_message(client, control_stream_handler, *m, context).await
    }
    ControlMessage::Switch(m) => {
      handle_switch_message(client, control_stream_handler, *m, context).await
    }
    _ => {
      // no-op
      Ok(())
    }
  }
}
