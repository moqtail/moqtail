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
use crate::server::message_handlers::parameters;
use crate::server::session::Session;
use crate::server::session_context::PendingRequest;
use crate::server::session_context::SessionContext;
use crate::server::subscription::Subscription;
use crate::server::track::{Track, TrackOrigin, TrackStatus};
use crate::server::track_manager::SubscribeKind;
use core::result::Result;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::{
  constant::GroupOrder, control_message::ControlMessage, publish::Publish,
  request_error::RequestError, request_ok::RequestOk,
};
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::error::{RequestErrorCode, TerminationCode};
use moqtail::model::parameter::constant::MessageParameterType;
use moqtail::model::parameter::message_parameter::apply_message_parameter_update;
use moqtail::model::parameter::message_parameter::{MessageParameter, MessageParameterVecExt};
use moqtail::model::property::track_property::has_unsupported_mandatory;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

pub async fn handle(
  client: Arc<MOQTClient>,
  stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
  opening_request_id: Option<u64>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::Publish(m) => {
      info!("Received Publish message for track: {:?}", m.track_name);
      let request_id = m.request_id;
      let track_alias = m.track_alias;

      // reject a PUBLISH with an unsupported mandatory track property.
      if has_unsupported_mandatory(&m.track_properties) {
        let reason_phrase =
          ReasonPhrase::try_new("Unsupported mandatory track property".to_string())
            .map_err(|_| TerminationCode::InternalError)?;
        let publish_error = Box::new(RequestError::new(
          RequestErrorCode::UnsupportedExtension,
          0,
          reason_phrase,
        ));
        return stream_handler
          .send(&ControlMessage::RequestError(publish_error))
          .await;
      }

      // Validate track namespace authorization
      // TODO: Implement actual authorization logic
      let is_authorized = validate_publish_authorization(&m.track_namespace, &client).await;

      if !is_authorized {
        let reason_phrase =
          ReasonPhrase::try_new("Not authorized to publish this track".to_string())
            .map_err(|_| TerminationCode::InternalError)?;

        let publish_error = Box::new(RequestError::new(
          RequestErrorCode::Unauthorized,
          0, //TODO: Maybe decide on another retry interval?
          reason_phrase,
        ));

        return stream_handler
          .send(&ControlMessage::RequestError(publish_error))
          .await;
      }

      // Build the full track name early so we can check it against any existing alias mapping.
      let full_track_name = moqtail::model::data::full_track_name::FullTrackName {
        namespace: m.track_namespace.clone(),
        name: m.track_name.clone(),
      };

      // Tracks MUST NOT be published under a reserved namespace.
      if let Some(reason) =
        crate::server::utils::reserved_namespace_rejection(&m.track_namespace, &m.track_name)
      {
        info!("Rejecting PUBLISH for reserved namespace: {}", reason);
        let publish_error = Box::new(RequestError::new(
          RequestErrorCode::DoesNotExist,
          0,
          ReasonPhrase::try_new(reason.to_string()).map_err(|_| TerminationCode::InternalError)?,
        ));
        return stream_handler
          .send(&ControlMessage::RequestError(publish_error))
          .await;
      }

      // Multiple publishers may share the same alias for the same track (fan-out).
      // Only reject if the alias already maps to a different full track name for this connection.
      {
        if context
          .track_manager
          .has_track_alias(context.connection_id, &m.track_alias)
          .await
        {
          let is_same_track = if let Some(existing) = context
            .track_manager
            .get_track_by_alias(context.connection_id, m.track_alias)
            .await
          {
            existing.read().await.full_track_name == full_track_name
          } else {
            false
          };
          if !is_same_track {
            return Err(TerminationCode::DuplicateTrackAlias);
          }
          // Same track, same alias — fall through to the has_track branch below.
        }
      }

      if !context.track_manager.has_track(&full_track_name).await {
        info!(
          "Track not found, creating new track for publisher alias={}",
          m.track_alias
        );
        let relay_track_id = context.track_manager.generate_relay_track_id();
        let track = Track::new(
          relay_track_id,
          full_track_name.clone(),
          context.server_config,
          TrackStatus::Confirmed {
            upstream_parameters: vec![],
          },
          TrackOrigin::Publish,
        );
        let track_arc = context
          .track_manager
          .add_track(
            context.connection_id,
            m.track_alias,
            full_track_name.clone(),
            track,
          )
          .await;
        {
          let track = track_arc.write().await;
          track
            .add_publisher(context.connection_id, track_alias)
            .await;
          track.set_track_properties(m.track_properties.clone()).await;
        }

        client
          .add_published_track(request_id, full_track_name.clone())
          .await;

        // register this publish message
        context
          .track_manager
          .add_publish_message(full_track_name.clone(), context.connection_id, (*m).clone())
          .await;

        {
          let mut map = client.inbound_requests.write().await;
          map.insert(
            request_id,
            PendingRequest::Publish {
              publisher_connection_id: context.connection_id,
              original_request_id: request_id,
              message: (*m).clone(),
            },
          );
        }

        let subscribers = context
          .track_manager
          .get_namespace_subscribers(&m.track_namespace, SubscribeKind::Tracks)
          .await;

        if !subscribers.is_empty() {
          info!(
            "Found {} subscribers for namespace {:?}, forwarding PUBLISH",
            subscribers.len(),
            m.track_namespace
          );
        }

        for (subscriber, subscribe_tracks_params) in subscribers {
          // Never push a track back to its own publisher, and let an explicit
          // SUBSCRIBE take precedence over SUBSCRIBE_TRACKS for the same track.
          if subscriber.connection_id == context.connection_id
            || track_arc
              .read()
              .await
              .get_subscription(subscriber.connection_id)
              .await
              .is_some()
          {
            continue;
          }

          info!(
            "Forwarding Publish to interested client: {}",
            subscriber.connection_id
          );

          let sub_clone = subscriber.clone();

          let mut m_clone = m.clone();

          let relay_req_id =
            Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

          m_clone.request_id = relay_req_id;
          m_clone.track_alias = relay_track_id;
          m_clone.parameters = parameters::downstream_publish(
            &m.parameters,
            &subscribe_tracks_params,
            track_arc.read().await.largest_object().await,
          );

          // Register the message in unified map for response tracking
          {
            let mut map = context.relay_pending_requests.write().await;
            map.insert(
              relay_req_id,
              PendingRequest::Publish {
                publisher_connection_id: context.connection_id,
                original_request_id: request_id,
                message: (*m_clone).clone(),
              },
            );
          }

          let track_write = track_arc.read().await;
          if let Err(e) = track_write
            .add_subscription(subscriber.clone(), (*m_clone).clone(), false)
            .await
          {
            warn!(
              "Failed to auto-subscribe client {} to pushed track: {:?}",
              subscriber.connection_id, e
            );
          }

          // Push the PUBLISH on its own bidi stream and read PUBLISH_OK there.
          let push_msg = *m_clone.clone();
          let subscription = track_write.get_subscription(subscriber.connection_id).await;
          tokio::spawn(async move {
            forward_publish_downstream(sub_clone, push_msg, subscription).await;
          });
        }
      } else {
        // Another publisher for the same track with a different alias.
        // Register their alias so their data stream can be routed to the existing track.
        context
          .track_manager
          .add_track_alias(
            context.connection_id,
            m.track_alias,
            full_track_name.clone(),
          )
          .await;
        if let Some(track_arc) = context.track_manager.get_track(&full_track_name).await {
          track_arc
            .write()
            .await
            .add_publisher(context.connection_id, m.track_alias)
            .await;
        }
        client
          .add_published_track(request_id, full_track_name.clone())
          .await;
        // Store this publisher's PUBLISH too. Raising the Forward State later needs the
        // request id it arrived on, and without it this publisher is skipped and never
        // told to start sending.
        context
          .track_manager
          .add_publish_message(full_track_name.clone(), context.connection_id, (*m).clone())
          .await;
        info!(
          "Additional publisher for existing track {:?}/{}: registered alias {}",
          m.track_namespace, m.track_name, m.track_alias
        );
      }

      let m_clone = m.clone();
      // FORWARD in PUBLISH is the publisher declaring its own behaviour: 0 means
      // it sends nothing until we raise the state, omitted or 1 means it sends
      // immediately. That declaration is the initial Forward State, but the reply
      // must be 1 whenever a downstream subscriber already wants Objects.
      let publisher_forwards = matches!(
        m_clone.parameters.get_param_or(
          MessageParameterType::Forward,
          MessageParameter::new_forward(true),
        ),
        MessageParameter::Forward { forward: true }
      );
      let forwarding = match context.track_manager.get_track(&full_track_name).await {
        Some(track_arc) => {
          let track = track_arc.read().await;
          let forwarding = publisher_forwards || track.has_forwarding_subscriber().await;
          track.upstream_forward.store(forwarding, Ordering::Relaxed);
          forwarding
        }
        None => publisher_forwards,
      };
      // PUBLISH is answered by REQUEST_OK (PUBLISH_OK); no Track Properties.
      // SUBSCRIBER_PRIORITY is omitted so the publisher uses the default of 128;
      // the relay has no reason to rank a pushed track above everything else.
      // SUBSCRIPTION_FILTER is omitted so the subscription is unfiltered: a relay
      // wants every Object, both to serve downstream filters and to fill its
      // cache for FETCH.
      let publish_ok = Box::new(RequestOk::new(vec![
        MessageParameter::new_forward(forwarding),
        MessageParameter::new_group_order(GroupOrder::Ascending),
      ]));

      info!(
        "Accepted publish request for track: {:?} with alias: {}",
        m_clone.track_name, m_clone.track_alias
      );

      stream_handler
        .send(&ControlMessage::RequestOk(publish_ok))
        .await
    }

    ControlMessage::PublishDone(m) => {
      // PUBLISH_DONE names no request; it ends the one that opened this stream.
      let Some(publisher_req_id) = opening_request_id else {
        warn!("PUBLISH_DONE on the control stream; closing session");
        return Err(TerminationCode::ProtocolViolation);
      };

      info!(
        "Received PublishDone message for request ID: {} with status: {:?}",
        publisher_req_id, m.status_code
      );

      // Clean up the published track
      cleanup_published_track(&client, publisher_req_id, &context).await;

      // Remove the request from the unified map to avoid memory leak
      {
        let mut map = client.inbound_requests.write().await;
        map.remove(&publisher_req_id);
        debug!(
          "Removed terminated PUBLISH request {} from pending requests map",
          publisher_req_id
        );
      }

      Ok(())
    }

    ControlMessage::RequestUpdate(m) => {
      let update_msg = *m;
      let Some(target_request_id) = opening_request_id else {
        return Err(TerminationCode::ProtocolViolation);
      };
      let publisher_req_id = target_request_id;

      {
        let mut map = client.inbound_requests.write().await;
        match map.get_mut(&publisher_req_id) {
          Some(PendingRequest::Publish { message, .. }) => {
            apply_message_parameter_update(&mut message.parameters, update_msg.parameters.clone());
          }
          _ => {
            warn!(
              "Request {} is not a valid Publish request",
              publisher_req_id
            );
            return Err(TerminationCode::ProtocolViolation);
          }
        }
      }

      // 2. Look up the track this publisher owns
      let full_track_name = match context
        .track_manager
        .get_track_name_by_publisher(client.connection_id, publisher_req_id)
        .await
      {
        Some(name) => name,
        None => {
          warn!(
            "No active track found for publisher request {}",
            publisher_req_id
          );
          return Err(TerminationCode::ProtocolViolation);
        }
      };

      let track_arc = match context.track_manager.get_track(&full_track_name).await {
        Some(t) => t,
        None => {
          warn!("Track metadata missing for {:?}", full_track_name);
          return Err(TerminationCode::InternalError);
        }
      };

      info!(
        "Processing Publish REQUEST_UPDATE for track {:?}",
        full_track_name
      );

      // 3. Update the Track's global metadata
      context
        .track_manager
        .update_publish_message_parameters(
          &full_track_name,
          client.connection_id,
          &update_msg.parameters,
        )
        .await;

      // 4. FAN-OUT: Translate the IDs and notify all downstream subscribers
      let active_subscriptions = {
        let track_read = track_arc.read().await;
        track_read
          .subscription_manager
          .get_all_subscriptions()
          .await
      };

      if active_subscriptions.is_empty() {
        info!(
          "No active subscribers for track {:?}, skipping fan-out.",
          full_track_name
        );
      } else {
        info!(
          "Fanning out Publish update to {} subscribers",
          active_subscriptions.len()
        );
      }

      for sub_lock in active_subscriptions {
        let sub = sub_lock.read().await;
        let subscriber_client = sub.subscriber().clone();

        let subscriber_existing_id = sub.request_id;

        let relay_update_id =
          Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

        let mut forwarded_update = update_msg.clone();
        forwarded_update.request_id = relay_update_id;

        // Goes on the subscription's own request stream, which is what names
        // the request being updated.
        subscriber_client
          .send_response(
            subscriber_existing_id,
            ControlMessage::RequestUpdate(Box::new(forwarded_update)),
          )
          .await;
      }

      use moqtail::model::control::request_ok::RequestOk;
      let ok_msg = RequestOk::new(vec![]);
      stream_handler
        .send(&ControlMessage::RequestOk(Box::new(ok_msg)))
        .await?;

      Ok(())
    }
    _ => Ok(()),
  }
}

/// Raise a PUBLISH-created track's upstream Forward State to 1 once something
/// downstream wants Objects. A publisher that declared FORWARD=0 sends nothing
/// until told to, so without this the track never delivers.
pub(crate) async fn ensure_upstream_forwarding(
  track_arc: &Arc<tokio::sync::RwLock<Track>>,
  context: &Arc<SessionContext>,
) {
  let (origin, full_track_name) = {
    let track = track_arc.read().await;
    (track.origin, track.full_track_name.clone())
  };
  if origin != TrackOrigin::Publish {
    return;
  }

  let publisher_ids: Vec<usize> = {
    let track = track_arc.read().await;
    if !track.has_forwarding_subscriber().await {
      return;
    }
    // swap returns the previous value, so false means this call is the one that
    // set the flag and therefore owns the update; true means someone beat us.
    if track.upstream_forward.swap(true, Ordering::Relaxed) {
      return;
    }
    let aliases = track.publisher_aliases.read().await;
    aliases.keys().copied().collect()
  };

  for connection_id in publisher_ids {
    let Some(publisher_request_id) = context
      .track_manager
      .get_publish_request_id(&full_track_name, connection_id)
      .await
    else {
      warn!("No stored PUBLISH for publisher {connection_id} on {full_track_name:?}");
      continue;
    };
    let publisher = { context.client_manager.get(connection_id).await };
    let Some(publisher) = publisher else {
      continue;
    };

    let update = moqtail::model::control::request_update::RequestUpdate::new(
      Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await,
      vec![MessageParameter::new_forward(true)],
    );
    // Goes on the publisher's own PUBLISH request stream, where the responses to
    // that request belong.
    let delivered = publisher
      .send_response(
        publisher_request_id,
        ControlMessage::RequestUpdate(Box::new(update)),
      )
      .await;
    if delivered {
      info!("Raised upstream Forward State for {full_track_name:?} (publisher {connection_id})");
    } else {
      warn!("No open PUBLISH request stream for publisher {connection_id} on {full_track_name:?}");
    }
  }
}

/// Push a PUBLISH to a subscriber on its own bidirectional request stream and read the
/// PUBLISH_OK (or REQUEST_ERROR) there; a rejection is logged.
pub(crate) async fn forward_publish_downstream(
  subscriber: Arc<MOQTClient>,
  publish: Publish,
  subscription: Option<Arc<RwLock<Subscription>>>,
) {
  let (send, recv) = match subscriber.connection.open_bi().await {
    Ok(streams) => streams,
    Err(e) => {
      error!("Failed to open downstream publish stream: {:?}", e);
      return;
    }
  };
  let mut stream = ControlStreamHandler::new(send, recv).with_peer_id(subscriber.connection_id);
  if let Err(e) = stream
    .send(&ControlMessage::Publish(Box::new(publish.clone())))
    .await
  {
    error!("Failed to push PUBLISH downstream: {:?}", e);
    return;
  }

  // The subscription is now live and can forward Objects; the PUBLISH_OK
  // (or REQUEST_ERROR) is just a status message and doesn't affect the subscription.
  if let Some(subscription) = &subscription {
    subscription.read().await.mark_alias_announced();
  }

  match stream.next_message().await {
    Ok(ControlMessage::RequestOk(ok)) => {
      register_ssts_assignment_from_publish_ok(
        subscriber.clone(),
        &publish,
        *ok,
        subscription.as_ref(),
      )
      .await;
      info!(
        "Pushed PUBLISH accepted by subscriber {}",
        subscriber.connection_id
      );
    }
    Ok(ControlMessage::RequestError(m)) => {
      warn!(
        "Subscriber {} rejected pushed PUBLISH: {:?}",
        subscriber.connection_id, m.error_code
      );
    }
    Ok(other) => warn!(
      "Unexpected {:?} on downstream publish stream",
      other.get_type()
    ),
    Err(_) => debug!("Downstream publish stream closed"),
  }
}

/// The subscriber may assign a track to an SSTS switching set by appending
/// SWITCHING_SET_ASSIGNMENT to the PUBLISH_OK that accepts a pushed PUBLISH
/// (draft-wilaw-moq-moqt-ssts, Section 4).
async fn register_ssts_assignment_from_publish_ok(
  subscriber: Arc<MOQTClient>,
  publish: &Publish,
  ok: RequestOk,
  subscription: Option<&Arc<RwLock<Subscription>>>,
) {
  let Some(p) = ok
    .parameters
    .iter()
    .find(|p| matches!(p, MessageParameter::SwitchingSetAssignment { .. }))
  else {
    return;
  };

  // The absence of the SSTS_ALGORITHMS setup option (or an empty list)
  // prohibits the use of SSTS (Section 3).
  if !subscriber.ssts_enabled {
    warn!(
      "Ignoring SWITCHING_SET_ASSIGNMENT in PUBLISH_OK from {}: SSTS was not negotiated in SETUP",
      subscriber.connection_id
    );
    return;
  }

  let Ok(full_track_name) =
    FullTrackName::new(publish.track_namespace.clone(), publish.track_name.clone())
  else {
    return;
  };

  // The pushed PUBLISH's track alias is the relay track id.
  let relay_track_id = publish.track_alias;

  if let MessageParameter::SwitchingSetAssignment {
    switching_set_id,
    algorithm_id,
    throughput_threshold_kbps,
    set_throughput_weight,
    activate_switching,
    set_rank,
  } = p
  {
    let mut manager = subscriber.switching_sets.write().await;
    if let Err(e) = manager.assign(
      full_track_name.clone(),
      relay_track_id,
      publish.request_id,
      *switching_set_id,
      *algorithm_id,
      *throughput_threshold_kbps,
      *set_throughput_weight,
      *activate_switching,
      *set_rank,
    ) {
      warn!(
        "Ignoring SWITCHING_SET_ASSIGNMENT in PUBLISH_OK from {}: {}",
        subscriber.connection_id, e
      );
      return;
    }

    // Same forward-state reasoning as the SUBSCRIBE path: the gating in the
    // subscription implements the spec's per-group forward selection.
    if let Some(subscription) = subscription {
      let sub = subscription.read().await;
      let mut state = sub.subscription_state.write().await;
      state.forward = true;
    }
  }
}

/// Validates if the client is authorized to publish to the given track namespace
async fn validate_publish_authorization(
  _track_namespace: &moqtail::model::common::tuple::Tuple,
  _client: &Arc<MOQTClient>,
) -> bool {
  // TODO: Implement actual authorization logic
  // This could check:
  // - Client authentication credentials
  // - Track namespace permissions
  // - Rate limiting
  // - Subscription quotas

  // For now, allow all publishes (this should be replaced with actual auth logic)
  true
}

/// Cleans up resources associated with a published track.
/// Removes the publisher from its track; if it was the last publisher,
/// remove_publisher() internally notifies subscribers.
async fn cleanup_published_track(
  client: &Arc<MOQTClient>,
  request_id: u64,
  context: &Arc<SessionContext>,
) {
  let full_track_name = {
    let published_tracks = client.published_tracks.read().await;
    published_tracks.get(&request_id).cloned()
  };

  let full_track_name = match full_track_name {
    Some(n) => n,
    None => {
      info!(
        "cleanup_published_track: no track found for request_id={}",
        request_id
      );
      return;
    }
  };

  let track_arc = match context.track_manager.get_track(&full_track_name).await {
    Some(t) => t,
    None => {
      info!(
        "cleanup_published_track: track not in manager for request_id={}",
        request_id
      );
      return;
    }
  };

  // SSTS: the upstream track is going away (PUBLISH_DONE); remove it from the
  // switching sets of all downstream subscribers (Section 6.3). Done before
  // the track may be dropped below.
  track_arc
    .read()
    .await
    .remove_from_subscriber_switching_sets()
    .await;

  let track = track_arc.read().await;
  if let Some(alias) = track.remove_publisher(client.connection_id).await {
    context
      .track_manager
      .remove_publisher_alias(client.connection_id, alias)
      .await;
    if !track.has_publishers().await {
      drop(track);
      context.track_manager.remove_track(&full_track_name).await;
      info!(
        "cleanup_published_track: removed track {:?} (no publishers left) request_id={}",
        full_track_name, request_id
      );
    }
  }
}
