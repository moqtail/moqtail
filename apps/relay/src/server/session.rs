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

use anyhow::Result;
use bytes::Bytes;
use moqtail::model::{
  control::{
    constant::SUPPORTED_VERSIONS, control_message::ControlMessage, server_setup::ServerSetup,
  },
  data::{constant::ObjectForwardingPreference, datagram::Datagram},
  error::TerminationCode,
};
use moqtail::transport::{
  control_stream_handler::ControlStreamHandler,
  data_stream_handler::{HeaderInfo, RecvDataStream},
};
use std::{
  collections::HashMap,
  sync::{Arc, atomic::AtomicU64},
};
use tokio::sync::RwLock;
use tracing::{Instrument, debug, error, info, info_span, warn};
use wtransport::{RecvStream, SendStream, endpoint::IncomingSession, error::ConnectionError};

use crate::server::{Server, stream_id::StreamId};

use super::{
  client::MOQTClient,
  message_handlers,
  session_context::{RequestMaps, SessionContext},
  track::Track,
  utils,
};

pub struct Session {}

impl Session {
  pub async fn new(incoming_session: IncomingSession, server: Server) -> Result<Session> {
    let session_request = incoming_session.await?;

    let remote_addr = session_request.remote_address();
    let origin = session_request.origin();
    let authority = session_request.authority();
    let path = session_request.path();
    let user_agent = session_request.user_agent();
    let headers = session_request.headers();

    info!(
      "New session:
      Remote Address: '{:?}'
      Origin: '{:?}'
      Authority: '{}', 
      Path: '{}'
      User-Agent: '{:?}' 
      Headers: '{:?}'
      ",
      remote_addr, origin, authority, path, user_agent, headers
    );

    let client_protocols = Self::parse_available_protocols(headers);
    info!(
      "Relay supported protocols: {} - Client supported protocols: {:?}",
      SUPPORTED_VERSIONS, client_protocols
    );

    let selected_version = match Self::select_protocol(&client_protocols) {
      Some(v) => {
        info!("Selected protocol version: {}", v);
        v.to_string()
      }
      None => {
        session_request.forbidden().await;
        return Err(anyhow::anyhow!(
          "No mutually supported protocol version found"
        ));
      }
    };

    let mut response_headers: HashMap<String, String> = HashMap::new();
    response_headers.insert("wt-protocol".to_string(), selected_version);

    // in the headers, we expect wt-available-protocols

    let client_manager = server.client_manager.clone();
    let track_manager = server.track_manager.clone();
    let server_config = server.app_config;
    let relay_pending_requests = server.relay_pending_requests.clone();
    let relay_next_request_id = server.relay_next_request_id.clone();
    let connection = session_request.accept().await?;

    let request_maps = RequestMaps {
      relay_pending_requests,
    };

    let context = Arc::new(SessionContext::new(
      server_config,
      client_manager,
      track_manager,
      request_maps,
      connection,
      relay_next_request_id,
    ));

    tokio::spawn(Self::handle_connection_close(context.clone()));
    tokio::spawn(Self::accept_control_stream(context.clone()));

    Ok(Session {})
  }

  async fn accept_control_stream(context: Arc<SessionContext>) -> Result<()> {
    match context.connection.accept_bi().await {
      Ok((send_stream, recv_stream)) => {
        let session_context = context.clone();
        let connection_id = session_context.connection_id;
        tokio::spawn(async move {
          if let Err(e) =
            Self::handle_control_messages(session_context.clone(), send_stream, recv_stream)
              .instrument(info_span!("handle_control_messages", connection_id))
              .await
          {
            match e {
              TerminationCode::NoError => {
                info!(
                  "Control stream ended due to client disconnect (connection_id: {})",
                  connection_id
                );
                // Client has already disconnected, no need to close connection
                session_context
                  .is_connection_closed
                  .store(true, std::sync::atomic::Ordering::Relaxed);
              }
              _ => {
                error!("Error processing control messages: {:?}", e);
                Self::close_session(context.clone(), e, "Error in control stream handler");
              }
            }
          }
        });
        Ok(())
      }
      Err(e) => {
        error!("Failed to accept stream: {:?}", e);
        Self::close_session(
          context.clone(),
          TerminationCode::InternalError,
          "Error in control stream handler",
        );
        Err(e.into())
      }
    }
  }

  async fn handle_control_messages(
    context: Arc<SessionContext>,
    send_stream: SendStream,
    recv_stream: RecvStream,
  ) -> core::result::Result<(), TerminationCode> {
    info!("new control message stream");
    let mut control_stream_handler = ControlStreamHandler::new(send_stream, recv_stream);

    // Client-server negotiation
    let client = match Self::negotiate(context.clone(), &mut control_stream_handler)
      .instrument(info_span!("negotiate", context.connection_id))
      .await
    {
      Ok(client) => client,
      Err(e) => {
        context.connection.close(0u32.into(), b"Setup failed");
        return Err(e);
      }
    };

    // Set the client in the context
    context.set_client(client.clone()).await;

    // start waiting for unistreams
    let session_context = context.clone();
    tokio::spawn(async move {
      let _ = Self::wait_for_streams(session_context).await;
    });

    // Message loop
    loop {
      // see if we have a message to receive from the client
      let msg: ControlMessage;
      let c = client.clone(); // this is the client that is connected to this session
      {
        tokio::select! {
          m = control_stream_handler.next_message() => {
            msg = match m {
              Ok(m) => {
                info!("received control message: {:?}", m);
                m
              },
              Err(TerminationCode::NoError) => {
                info!("Client disconnected, ending control message loop");
                // Client has already disconnected, no need to close connection
                return Err(TerminationCode::NoError);
              }
              Err(e) => {
                error!("failed to deserialize message: {:?}", e);
                return Err(e);
              }
            };
          },
          m = c.wait_for_next_message() => {
            msg = m;
            info!("new message for client: {:?}", msg);
            if let Err(e) = control_stream_handler.send(&msg).await {
              error!("Error sending message: {:?}", e);
              return Err(e);
            }
            continue;
          },
          else => {
            info!("no message received");
            continue;
          },
        } // end of tokio::select!
      }

      let message_type = &msg.get_type();

      let handling_result = message_handlers::MessageHandler::handle(
        client.clone(),
        &mut control_stream_handler,
        msg,
        context.clone(),
      )
      .await;

      if let Err(termination_code) = handling_result {
        error!(
          "Error handling {} message: {:?}",
          termination_code, message_type
        );
        Self::close_session(
          context.clone(),
          termination_code,
          "Error handling PublishNamespace message",
        );
        return Err(termination_code);
      }
    } // end of loop
  }

  fn close_session(context: Arc<SessionContext>, error_code: TerminationCode, msg: &str) {
    context
      .connection
      .close(error_code.to_u32().into(), msg.as_bytes());
  }

  // TODO: in an error close the connection
  async fn wait_for_streams(context: Arc<SessionContext>) -> Result<()> {
    info!(
      "wait_for_streams | connection id: {}",
      context.connection_id
    );
    loop {
      let session_context = context.clone();
      if session_context
        .is_connection_closed
        .load(std::sync::atomic::Ordering::Relaxed)
      {
        info!(
          "Connection closed, stopping stream acceptance for connection {}",
          session_context.connection_id
        );
        return Ok(());
      }
      tokio::select! {
        stream = context.connection.accept_bi() => {
          match stream {
            Ok((send, recv)) => {
              let ctx = context.clone();
              tokio::spawn(async move {
                Self::handle_request_stream(ctx, send, recv).await;
              });
            }
            Err(e) => {
              error!("Failed to accept bi-stream: {:?}", e);
            }
          }
        }

        stream = context.connection.accept_uni() => {
          tokio::spawn(async move {
            let stream = match stream {
              Ok(stream) => {
                stream
              },
              Err(e) => {
                // TODO: do we need to close the connection here?
                error!("Failed to accept unidirectional stream: {:?}", e);
                return;
              }
            };

            Self::handle_uni_stream(
              session_context,
              stream,
            ).await.unwrap_or_else(|e| {
              error!("Error processing unidirectional stream: {:?}", e);
            });
          });
        }

        datagram_result = context.connection.receive_datagram() => {
          let connection_id = context.connection_id;
          let context_clone = Arc::clone(&context);
          let datagram_bytes = match datagram_result {
            Ok(bytes) => bytes,
            Err(e) => {
              if matches!(e, ConnectionError::ApplicationClosed(_)) {
                continue; // Connection closed, exit the loop
              }
              error!("Failed to receive datagram from client {}: {:?}", connection_id, e);
              continue;
            }
          };

          tokio::spawn(async move {
            debug!("Received datagram from client {}", connection_id);

            // Parse the datagram
            let bytes = Bytes::from(datagram_bytes.payload().to_vec());
            let mut bytes = bytes;
            match Datagram::deserialize(&mut bytes) {
              Ok(datagram_obj) => {
                debug!("Parsed datagram: track_alias={}, group_id={}, object_id={}",
                       datagram_obj.track_alias, datagram_obj.group_id, datagram_obj.object_id);

                if let Some(track) = context_clone.track_manager.get_track_by_alias(context_clone.connection_id, datagram_obj.track_alias).await {
                  let track = track.read().await;
                  // Set forwarding preference to Datagram
                  track.set_forwarding_preference(ObjectForwardingPreference::Datagram).await;

                  // Call new_datagram
                  if let Err(e) = track.new_datagram(&datagram_obj).await {
                    error!("Failed to process datagram: {:?}", e);
                  }
                } else {
                  debug!("Track not found for track_alias {}", datagram_obj.track_alias);
                }
              }
              Err(e) => {
                error!("Failed to parse datagram: {:?}", e);
              }
            }
          });
        }
      }
    }
  }

  async fn handle_request_stream(
    context: Arc<SessionContext>,
    send_stream: SendStream,
    recv_stream: RecvStream,
  ) {
    let client = match context.get_client().await {
      Some(c) => c,
      None => return,
    };
    let mut stream_handler = ControlStreamHandler::new(send_stream, recv_stream);

    let msg = match stream_handler.next_message().await {
      Ok(m) => m,
      Err(e) => {
        error!("Failed to read first message on request bi-stream: {:?}", e);
        return;
      }
    };

    // Validate request_id
    let request_id_opt = match &msg {
      ControlMessage::SubscribeNamespace(m) => Some(m.request_id),
      _ => None,
    };
    if let Some(rid) = request_id_opt {
      let max_request_id = context
        .max_request_id
        .load(std::sync::atomic::Ordering::Relaxed);
      if rid >= max_request_id {
        warn!(
          "Request bi-stream: request_id ({}) >= max_request_id ({})",
          rid, max_request_id
        );
        Self::close_session(
          context,
          TerminationCode::TooManyRequests,
          "Request ID exceeds maximum",
        );
        return;
      }
    }

    Self::dispatch_request_stream_message(client, stream_handler, msg, context).await;
  }

  async fn dispatch_request_stream_message(
    client: Arc<MOQTClient>,
    mut stream_handler: ControlStreamHandler,
    msg: ControlMessage,
    context: Arc<SessionContext>,
  ) {
    match msg {
      ControlMessage::SubscribeNamespace(sub_ns) => {
        let request_id = sub_ns.request_id;

        let (namespace_tx, mut namespace_rx) =
          tokio::sync::mpsc::unbounded_channel::<ControlMessage>();

        let result = message_handlers::subscribe_namespace_handler::handle_subscribe_namespace(
          client.clone(),
          &mut stream_handler,
          sub_ns,
          context.clone(),
          namespace_tx,
        )
        .await;

        if let Err(e) = result {
          error!("handle_subscribe_namespace error: {:?}", e);
          return;
        }

        // Long-lived loop: forward NAMESPACE messages to bi-stream, or cancel on stream close
        loop {
          tokio::select! {
            biased;
            msg_opt = namespace_rx.recv() => {
              match msg_opt {
                Some(ns_msg) => {
                  if let Err(e) = stream_handler.send(&ns_msg).await {
                    warn!("Error writing NAMESPACE to bi-stream: {:?}", e);
                    message_handlers::subscribe_namespace_handler::cancel(client, request_id, &context).await;
                    return;
                  }
                }
                None => return, // channel dropped, subscription ended
              }
            }
            result = stream_handler.next_message() => {
              match result {
                Ok(extra) => {
                  warn!("Unexpected message {:?} on request bi-stream", extra.get_type());
                  Self::close_session(context, TerminationCode::ProtocolViolation, "Unexpected message on request bi-stream");
                  return;
                }
                Err(_) => {
                  // FIN or RESET — subscriber cancelled
                  message_handlers::subscribe_namespace_handler::cancel(client, request_id, &context).await;
                  return;
                }
              }
            }
          }
        }
      }
      _ => {
        warn!(
          "Unsupported message type {:?} on request bi-stream",
          msg.get_type()
        );
        Self::close_session(
          context,
          TerminationCode::ProtocolViolation,
          "Unsupported message type on request bi-stream",
        );
      }
    }
  }

  async fn handle_connection_close(context: Arc<SessionContext>) -> Result<()> {
    let client_manager_cleanup = context.client_manager.clone();
    let track_manager_cleanup = context.track_manager.clone();

    debug!(
      "handle_connection_close | waiting ({})",
      context.connection_id
    );
    context.connection.closed().await;

    // set the connection closed flag
    {
      context
        .is_connection_closed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    info!(
      "handle_connection_close | connection closed ({})",
      context.connection_id
    );

    // Remove the disconnecting publisher from all tracks they were publishing.
    // remove_publisher() internally notifies subscribers when the last publisher leaves.
    // Collect full track names for tracks that have no publishers left.
    let mut tracks_with_no_publishers: Vec<moqtail::model::data::full_track_name::FullTrackName> =
      Vec::new();
    {
      let tracks = track_manager_cleanup.tracks.read().await;
      for (full_track_name, track_lock) in tracks.iter() {
        let track = track_lock.read().await;
        if let Some(alias) = track.remove_publisher(context.connection_id).await {
          track_manager_cleanup
            .remove_publisher_alias(context.connection_id, alias)
            .await;
          if !track.has_publishers().await {
            tracks_with_no_publishers.push(full_track_name.clone());
          }
        }
      }
    }

    // Remove tracks that have no publishers left
    for full_track_name in &tracks_with_no_publishers {
      track_manager_cleanup.remove_track(full_track_name).await;
      info!(
        "Removed track {:?} after publisher {} disconnect",
        full_track_name, context.connection_id
      );
    }

    // Remove announcements for the disconnecting publisher
    track_manager_cleanup
      .remove_announcements_by_connection(context.connection_id)
      .await;

    // Remove the disconnecting client from namespace_subscribers
    track_manager_cleanup
      .remove_namespace_subscriber(context.connection_id)
      .await;

    // Remove client from client_manager
    {
      let cm = client_manager_cleanup.read().await;
      cm.remove(context.connection_id).await;
    }
    debug!(
      "handle_connection_close | removed client {} from client manager",
      context.connection_id
    );

    // Remove client from all remaining tracks (as a subscriber)
    for (_, track_lock) in track_manager_cleanup.tracks.read().await.iter() {
      let track = track_lock.read().await;
      track.remove_subscription(context.connection_id).await;
    }

    info!(
      "handle_connection_close | cleanup done ({})",
      context.connection_id
    );
    Ok(())
  }

  async fn handle_uni_stream(context: Arc<SessionContext>, stream: RecvStream) -> Result<()> {
    debug!("accepted unidirectional stream");
    let client = context.get_client().await;
    let client = match client {
      Some(c) => c,
      None => return Err(TerminationCode::InternalError.into()),
    };

    debug!("client is {}", client.connection_id);

    let mut stream_handler = &RecvDataStream::new(stream, client.fetch_requests.clone());

    let mut first_object = true;
    let mut track_alias = 0u64;
    let mut stream_id: Option<StreamId> = None;
    let mut current_track: Option<Arc<RwLock<Track>>> = None;

    let mut object_count = 0;

    loop {
      let next = stream_handler.next_object().await;

      match next {
        (handler, Some(object)) => {
          // Handle the object
          stream_handler = handler;

          let header_info = if first_object {
            // debug!("First object received, processing header info");
            let header = handler.get_header_info().await;
            if header.is_none() {
              error!("no header info found, terminating session");
              return Err(anyhow::Error::msg(TerminationCode::InternalError.to_json()));
            }

            // Unwrap the header info
            let header_info = header.unwrap();

            match header_info {
              HeaderInfo::Subgroup { header } => {
                debug!("received Subgroup header: {:?}", header);
                track_alias = header.track_alias;
              }
              HeaderInfo::Fetch {
                header,
                fetch_request: _,
              } => {
                debug!("received Fetch header: {:?}", header);
                let fetch_request_id = header.request_id;
                track_alias = client
                  .fetch_requests
                  .read()
                  .await
                  .get(&fetch_request_id)
                  .map_or(0, |r| r.track_alias);
              }
            }

            // The track might have not been added yet, wait for it for a few attempts
            let mut attempt_count = 5;
            let attempt_interval_ms = 100;
            debug!("looking for track with alias: {:?}", track_alias);
            loop {
              if attempt_count == 0 {
                error!("track not found after multiple attempts: {:?}", track_alias);
                return Err(anyhow::Error::msg(TerminationCode::InternalError.to_json()));
              }
              let full_track_name_opt = context
                .track_manager
                .track_aliases
                .read()
                .await
                .get(&(client.connection_id, track_alias))
                .cloned();

              if full_track_name_opt.is_none() {
                attempt_count -= 1;
                tokio::time::sleep(std::time::Duration::from_millis(attempt_interval_ms)).await;
                continue;
              }

              current_track = if let Some(full_track_name) = full_track_name_opt {
                if let Some(t) = context.track_manager.get_track(&full_track_name).await {
                  debug!("track found: {:?}", track_alias);
                  Some(t)
                } else {
                  error!(
                    "track not found: {:?} full track name: {:?}",
                    track_alias, &full_track_name
                  );
                  return Err(anyhow::Error::msg(TerminationCode::InternalError.to_json()));
                }
              } else {
                None
              };
              break;
            }

            let relay_track_id = match current_track.as_ref() {
              Some(t) => t.read().await.relay_track_id,
              None => {
                error!("track not found when building stream_id: {:?}", track_alias);
                return Err(anyhow::Error::msg(TerminationCode::InternalError.to_json()));
              }
            };
            stream_id = Some(utils::build_stream_id(relay_track_id, &header_info));
            Some(header_info)
          } else {
            None
          };

          let track = current_track.as_ref().unwrap().read().await;

          if let Err(e) = track
            .new_subgroup_object(
              &stream_id.clone().unwrap().clone(),
              &object,
              header_info.as_ref(),
            )
            .await
          {
            warn!(
              "Failed to process object track: alias: {:?} track name: {:?} error: {:?}",
              track_alias, &track.full_track_name, e
            );
          }

          object_count += 1;
          first_object = false; // reset the first object flag after processing the header
        }
        (_, None) => {
          // error!("Failed to receive object: {:?}", e);
          info!(
            "no more objects in the stream client: {} track: {} stream_id: {:?} objects: {}",
            context.connection_id,
            track_alias,
            stream_id.clone(),
            object_count
          );
          // Close the stream for all subscribers
          if let Some(track_lock) = &current_track {
            return track_lock
              .read()
              .await
              .stream_closed(&stream_id.unwrap())
              .await;
          }
          break;
        }
      }
    }

    Ok(())
  }

  pub(crate) async fn get_next_relay_request_id(relay_next_request_id: Arc<AtomicU64>) -> u64 {
    relay_next_request_id.fetch_add(2, std::sync::atomic::Ordering::Relaxed)
  }

  fn parse_available_protocols(headers: &HashMap<String, String>) -> Vec<String> {
    headers
      .get("wt-available-protocols")
      .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
      .unwrap_or_default()
  }

  fn select_protocol(client_protocols: &[String]) -> Option<&'static str> {
    // wt available protocols are surrounded by quotes
    SUPPORTED_VERSIONS.split(',').find(|v| {
      client_protocols
        .iter()
        .any(|p| p.as_str() == format!("\"{}\"", v))
    })
  }

  async fn negotiate(
    context: Arc<SessionContext>,
    control_stream_handler: &mut ControlStreamHandler,
  ) -> Result<Arc<MOQTClient>, TerminationCode> {
    debug!("Negotiating with client...");
    let client_setup = match control_stream_handler.next_message().await {
      Ok(ControlMessage::ClientSetup(m)) => *m,
      Ok(_) => {
        error!("Unexpected message received");
        return Err(TerminationCode::ProtocolViolation);
      }
      Err(TerminationCode::NoError) => {
        info!("Client disconnected during negotiation");
        return Err(TerminationCode::NoError);
      }
      Err(e) => {
        error!("Failed to deserialize message: {:?}", e);
        return Err(e);
      }
    };

    utils::print_msg_bytes(&client_setup);

    let max_request_id_param = {
      let max_request_id = context
        .max_request_id
        .load(std::sync::atomic::Ordering::Relaxed);
      moqtail::model::parameter::setup_parameter::SetupParameter::new_max_request_id(
        max_request_id + 1,
      )
      .try_into()
      .unwrap()
    };

    let moqt_implementation_param =
      moqtail::model::parameter::setup_parameter::SetupParameter::new_moqt_implementation(
        env!("MOQTAIL_VERSION").to_string(),
      )
      .try_into()
      .unwrap();

    let server_setup = ServerSetup::new(vec![max_request_id_param, moqt_implementation_param]);

    debug!("client setup: {:?}", client_setup);
    debug!("server setup: {:?}", server_setup);

    // Check for authorization tokens and log them if token logging is enabled
    if context.server_config.enable_token_logging {
      // Import the necessary types
      use moqtail::model::parameter::constant::TokenAliasType;
      use moqtail::model::parameter::setup_parameter::SetupParameter;

      // Iterate through setup parameters to find authorization tokens
      for param in &client_setup.setup_parameters {
        // Try to deserialize as a SetupParameter
        if let Ok(setup_param) = SetupParameter::deserialize(param) {
          // Check if it's an AuthorizationToken
          if let SetupParameter::AuthorizationToken { token } = setup_param
            && token.alias_type == TokenAliasType::Register as u64
          {
            // Get client port number from the connection
            let client_port = context.connection.remote_address().port();

            info!(
              "Authorization token registered: {:?}, port: {:?}",
              token, client_port
            );

            // Log the token information
            if let Some(token_value) = token.token_value {
              super::token_logger::log_token_registration(
                &token_value,
                context.connection_id,
                client_port,
                &context.server_config.token_log_path,
              );
            }
          }
        }
      }
    }

    let mut m = context.client_manager.write().await;

    let client = MOQTClient::new(
      context.connection_id,
      Arc::new(context.connection.clone()),
      Arc::new(client_setup),
    );
    let client = Arc::new(client);
    m.add(client.clone()).await;

    match control_stream_handler.send_impl(&server_setup).await {
      Ok(_) => {
        debug!("Sent server setup to client");
        Ok(client)
      }
      Err(e) => {
        error!("Failed to send server setup: {:?}", e);
        Err(TerminationCode::InternalError)
      }
    }
  }
}
