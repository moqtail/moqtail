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
use crate::server::session::Session;
use crate::server::session_context::SessionContext;
use crate::server::track::Track;
use core::result::Result;
use moqtail::model::error::TerminationCode;
use moqtail::model::{
  common::reason_phrase::ReasonPhrase, control::control_message::ControlMessage,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::SubscribeRequest;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub async fn handle(
  client: Arc<MOQTClient>,
  control_stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::Subscribe(m) => {
      info!("received Subscribe message: {:?}", m);
      let sub = *m;
      let track_namespace = sub.track_namespace.clone();
      let request_id = sub.request_id;
      let full_track_name = sub.get_full_track_name();

      // check request id
      {
        let max_request_id = context.max_request_id.read().await;
        if request_id >= *max_request_id {
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

      let res: Result<(), TerminationCode> = {
        let track_opt = context.track_manager.get_track(&full_track_name).await;
        if let Some(track) = track_opt {
          info!("track already exists, sending SubscribeOk");
          let track = track.write().await;
          let res = track.add_subscription(client.clone(), sub.clone()).await;
          match res {
            Ok(_) => {
              info!(
                "subscription added successfully subscriber: {} track: {:?}",
                &client.connection_id, &track.track_alias
              );

              // TODO: Send the first sub_ok message to the subscriber
              // for now, just sending some default values
              let subscribe_ok =
                moqtail::model::control::subscribe_ok::SubscribeOk::new_ascending_with_content(
                  sub.request_id,
                  track.track_alias,
                  0,
                  None,
                  None,
                );

              control_stream_handler.send_impl(&subscribe_ok).await
            }
            Err(e) => {
              error!(
                "error adding subscription: subscriber: {} track: {:?} error: {:?}",
                &client.connection_id, &track.track_alias, e
              );
              Err(TerminationCode::InternalError)
            }
          }
        } else {
          info!(
            "Track not found, creating new track: {:?}",
            &full_track_name
          );

          // send the subscribe message to the publisher
          let mut new_sub = sub.clone();
          // relay wants to get data but it does not forward data to subscribers that has forward = false
          new_sub.forward = true;
          new_sub.request_id =
            Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

          publisher
            .queue_message(ControlMessage::Subscribe(Box::new(new_sub.clone())))
            .await;

          // insert this request id into the relay's subscribe requests
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
          Ok(())
        }
      };

      // return if there's an error
      if res.is_ok() {
        // insert this request id into the clients subscribe requests
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
    ControlMessage::SubscribeOk(m) => {
      info!("received SubscribeOk message: {:?}", m);
      let msg = *m;

      // this comes from the publisher
      // it should be sent to the subscriber
      let request_id = msg.request_id;

      let sub_request = {
        let requests = context.relay_subscribe_requests.read().await;
        // print out every request
        debug!("current requests: {:?}", requests);
        match requests.get(&request_id) {
          Some(m) => {
            info!("request id is verified: {:?}", request_id);
            m.clone()
          }
          None => {
            warn!("request id is not verified: {:?}", request_id);
            return Ok(());
          }
        }
      };

      // TODO: honor the values in the subscribe_ok message like
      // expires, group_order, content_exists, largest_location

      // now we're ready to send the subscribe_ok message to the subscriber
      let subscribe_ok =
        moqtail::model::control::subscribe_ok::SubscribeOk::new_ascending_with_content(
          sub_request.original_request_id,
          msg.track_alias,
          msg.expires,
          msg.largest_location,
          None,
        );
      // send the subscribe_ok message to the subscriber
      let subscriber = {
        let mngr = context.client_manager.read().await;
        mngr.get(sub_request.requested_by).await
      };

      if subscriber.is_none() {
        warn!("subscriber not found");
        return Ok(());
      }

      debug!("subscriber found: {:?}", sub_request.requested_by);
      let subscriber = subscriber.unwrap();

      info!(
        "sending SubscribeOk to subscriber: {:?}, msg: {:?}",
        sub_request.requested_by, &subscribe_ok
      );

      subscriber
        .queue_message(ControlMessage::SubscribeOk(Box::new(subscribe_ok)))
        .await;

      // create the track here if it doesn't exist
      let full_track_name = sub_request.original_subscribe_request.get_full_track_name();

      if !context.track_manager.has_track(&full_track_name).await {
        info!("Track not found, creating new track: {:?}", msg.track_alias);
        // subscribed_tracks.insert(sub.track_alias, Track::new(sub.track_alias, track_namespace.clone(), sub.track_name.clone()));
        let track = Track::new(
          msg.track_alias,
          sub_request
            .original_subscribe_request
            .track_namespace
            .clone(),
          sub_request.original_subscribe_request.track_name.clone(),
          context.connection_id,
          context.server_config,
        );
        let track = context
          .track_manager
          .add_track(msg.track_alias, full_track_name.clone(), track)
          .await;

        let res = track
          .read()
          .await
          .add_subscription(
            client.clone(),
            sub_request.original_subscribe_request.clone(),
          )
          .await;
        match res {
          Ok(_) => {
            info!(
              "subscription added successfully subscriber: {} track: {:?}",
              &subscriber.connection_id, &msg.track_alias
            );
          }
          Err(e) => {
            error!(
              "error adding subscription: subscriber: {} track: {:?} error: {:?}",
              &subscriber.connection_id, &msg.track_alias, e
            );
          }
        }
      }
      Ok(())
    }
    ControlMessage::Unsubscribe(m) => {
      info!("received Unsubscribe message: {:?}", m);
      // stop sending objects for the track for the subscriber
      // by removing the subscription
      // find the track alias by using the request id
      let requests = client.subscribe_requests.read().await;
      let request = requests.get(&m.request_id);
      if request.is_none() {
        // a warning is enough
        warn!("request not found for request id: {:?}", m.request_id);
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
      Ok(())
    }
    ControlMessage::SubscribeUpdate(m) => {
      info!("received SubscribeUpdate message: {:?}", m);

      // a subscribe update message contains subscription_request_id
      // which is the request id of the subscription we want to update
      let sub_request_id = m.subscription_request_id;

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
        match sub.update_subscription(*m).await {
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
    _ => {
      // no-op
      Ok(())
    }
  }
}
