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

use crate::cli::DeliveryMode;
use crate::connection::MoqConnection;
use crate::stats::ReceptionStats;
use crate::utils::should_log;
use anyhow::Result;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant::{FetchType, GroupOrder};
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::Fetch;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::datagram::Datagram;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::transport::connection::TransportConnection;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::RecvDataStream;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// Subscribe to one track on its own bidirectional request stream. Returns the
/// assigned track alias and the request-stream handler, which the caller keeps
/// alive for the subscription's lifetime.
async fn subscribe_track(
  connection: &Arc<TransportConnection>,
  namespace: &str,
  track_name: &str,
  request_id: u64,
  subscriber_priority: u8,
  group_order: GroupOrder,
  forward: bool,
) -> Result<(u64, ControlStreamHandler)> {
  let ns = Tuple::from_utf8_path(namespace);
  info!(
    "Subscribing to track: {}/{} (request_id={}, priority={}, forward={})",
    namespace, track_name, request_id, subscriber_priority, forward
  );
  let subscribe = Subscribe::new_latest_object(
    request_id,
    ns,
    TupleField::from_utf8(track_name),
    vec![
      MessageParameter::new_subscriber_priority(subscriber_priority),
      MessageParameter::new_group_order(group_order),
      MessageParameter::new_forward(forward),
    ],
  );

  // A request opens its own bidi stream, beginning with SUBSCRIBE; the response
  // comes back on the same stream.
  let (send, recv) = connection.open_bi().await?;
  let mut request_stream = ControlStreamHandler::new(send, recv);
  request_stream
    .send(&ControlMessage::Subscribe(Box::new(subscribe)))
    .await
    .map_err(|e| anyhow::anyhow!("Failed to send SUBSCRIBE: {:?}", e))?;

  match request_stream.next_message().await {
    Ok(ControlMessage::SubscribeOk(m)) => {
      info!(
        "Subscribed: track={} track_alias={}",
        track_name, m.track_alias
      );
      Ok((m.track_alias, request_stream))
    }
    Ok(m) => anyhow::bail!("Expected SubscribeOk for {}, got {:?}", track_name, m),
    Err(e) => anyhow::bail!("Failed waiting for SubscribeOk for {}: {:?}", track_name, e),
  }
}

pub struct SubscribeConfig {
  pub namespace: String,
  pub track_name: String,
  pub delivery_mode: DeliveryMode,
  pub duration: u64,
  pub subscriber_priority: u8,
  pub group_order: GroupOrder,
  pub extra_track: Option<(String, u8)>,
  pub forward: bool,
  pub joining_fetch: bool,
  pub joining_start: u64,
  pub joining_type: FetchType,
}

/// Issue a Joining FETCH referencing an existing subscription and log the
/// response (FETCH_OK when accepted, REQUEST_ERROR — e.g. INVALID_RANGE for a
/// non-forwarding subscription — when rejected).
async fn send_joining_fetch(
  connection: &Arc<TransportConnection>,
  joining_request_id: u64,
  joining_start: u64,
  fetch_type: FetchType,
) -> Result<()> {
  let request_id = 2u64;
  let parameters = vec![MessageParameter::new_subscriber_priority(200)];
  let fetch = Fetch::new_joining(
    request_id,
    fetch_type,
    joining_request_id,
    joining_start,
    parameters,
  )
  .map_err(|e| anyhow::anyhow!("Failed to build joining FETCH: {}", e))?;

  info!(
    "Sending Joining FETCH: type={:?} joining_request_id={} joining_start={}",
    fetch_type, joining_request_id, joining_start
  );

  let (send, recv) = connection.open_bi().await?;
  let mut request_stream = ControlStreamHandler::new(send, recv);
  request_stream
    .send(&ControlMessage::Fetch(Box::new(fetch)))
    .await
    .map_err(|e| anyhow::anyhow!("Failed to send joining FETCH: {:?}", e))?;

  match request_stream.next_message().await {
    Ok(ControlMessage::FetchOk(m)) => {
      info!(
        "Joining FETCH accepted: FetchOk end_location={:?}",
        m.end_location
      );
    }
    Ok(ControlMessage::RequestError(m)) => {
      error!("Joining FETCH rejected: RequestError {:?}", m);
    }
    Ok(m) => info!("Joining FETCH response: {:?}", m),
    Err(e) => error!("Joining FETCH: error reading response: {:?}", e),
  }
  Ok(())
}

pub async fn run(moq: MoqConnection, config: SubscribeConfig) -> Result<()> {
  // Keep `moq` alive for the whole function: its control stream carries only
  // SETUP now, but must stay open for the session's lifetime. Subscriptions use
  // their own bidi request streams; objects arrive on uni streams.
  let connection = moq.connection.clone();

  // Each subscription's request stream is held open for its lifetime.
  let mut request_streams = Vec::new();

  let (track_alias, primary_stream) = subscribe_track(
    &connection,
    &config.namespace,
    &config.track_name,
    0,
    config.subscriber_priority,
    config.group_order,
    config.forward,
  )
  .await?;
  request_streams.push(primary_stream);

  // Issue a Joining FETCH against the primary subscription (request_id 0). Kept
  // before the receive loop so its response is observed directly.
  if config.joining_fetch {
    send_joining_fetch(&connection, 0, config.joining_start, config.joining_type).await?;
    drop(request_streams);
    return Ok(());
  }

  let extra_alias = if let Some((ref extra_name, extra_priority)) = config.extra_track {
    let (alias, extra_stream) = subscribe_track(
      &connection,
      &config.namespace,
      extra_name,
      1,
      extra_priority,
      config.group_order,
      config.forward,
    )
    .await?;
    request_streams.push(extra_stream);
    Some((extra_name.clone(), alias))
  } else {
    None
  };

  let result = match config.delivery_mode {
    DeliveryMode::Datagram => receive_datagrams(&connection, track_alias, config.duration).await,
    DeliveryMode::Subgroup => {
      receive_streams(&connection, track_alias, extra_alias, config.duration).await
    }
  };
  drop(request_streams);
  result
}

async fn receive_datagrams(
  connection: &Arc<TransportConnection>,
  track_alias: u64,
  duration: u64,
) -> Result<()> {
  info!("Listening for datagrams...");

  let connection_clone = connection.clone();
  let datagram_task = tokio::spawn(async move {
    let mut stats = ReceptionStats::new();

    loop {
      match connection_clone.receive_datagram().await {
        Ok(datagram) => {
          let mut bytes_mut = datagram.clone();

          match Datagram::deserialize(&mut bytes_mut) {
            Ok(obj) => {
              if obj.track_alias != track_alias {
                debug!(
                  "Ignoring datagram for different track_alias: {}",
                  obj.track_alias
                );
                continue;
              }

              // Sanity check
              if obj.group_id >= 10000 || obj.object_id >= 10000 {
                error!(
                  "Invalid datagram values: group={}, object={}",
                  obj.group_id, obj.object_id
                );
                stats.record_parse_error();
                continue;
              }

              let sequence_ok = stats.record_object(obj.group_id, obj.object_id);

              if should_log(stats.total_received) || !sequence_ok {
                info!(
                  "Received datagram {}: group={}, object={}, size={} bytes, elapsed={}ms, seq={}",
                  stats.total_received,
                  obj.group_id,
                  obj.object_id,
                  obj.payload.as_ref().map_or(0, |p| p.len()),
                  stats.elapsed_ms(),
                  if sequence_ok { "OK" } else { "GAP" }
                );
              } else {
                debug!(
                  "Received datagram {}: group={}, object={}, seq=OK",
                  stats.total_received, obj.group_id, obj.object_id
                );
              }
            }
            Err(e) => {
              error!("Failed to parse datagram: {:?}", e);
              stats.record_parse_error();
            }
          }
        }
        Err(e) => {
          info!("Datagram receive ended: {:?}", e);
          break;
        }
      }
    }

    stats.report();
    stats
  });

  if duration > 0 {
    tokio::time::sleep(tokio::time::Duration::from_secs(duration)).await;
    info!("Duration elapsed, closing connection...");
    connection.close(0u32, b"Done");
  }

  let stats = datagram_task.await?;
  info!(
    "Subscriber finished: received={}, errors={}, gaps={}",
    stats.total_received, stats.parse_errors, stats.sequence_gaps
  );

  Ok(())
}

async fn receive_streams(
  connection: &Arc<TransportConnection>,
  primary_alias: u64,
  extra_alias: Option<(String, u64)>,
  duration: u64,
) -> Result<()> {
  info!("Listening for incoming streams...");

  // Build a map from track_alias → label for log output
  let mut alias_to_label = std::collections::HashMap::new();
  alias_to_label.insert(primary_alias, format!("alias={primary_alias}(primary)"));
  if let Some((ref name, alias)) = extra_alias {
    alias_to_label.insert(alias, format!("alias={alias}({name})"));
  }
  let alias_to_label = Arc::new(alias_to_label);

  let pending_fetches = Arc::new(RwLock::new(BTreeMap::new()));
  let conn = connection.clone();
  let pending_fetches_clone = pending_fetches.clone();

  let stream_task = tokio::spawn(async move {
    let mut stats = ReceptionStats::new();

    loop {
      match conn.accept_uni().await {
        Ok(stream) => {
          let stream_handler = RecvDataStream::new(stream, pending_fetches_clone.clone());
          let mut handler = &stream_handler;

          loop {
            let (next_handler, object) = handler.next_object().await;
            match object {
              Some(obj) => {
                let sequence_ok = stats.record_object(obj.location.group, obj.location.object);
                let label = alias_to_label
                  .get(&obj.track_alias)
                  .map(|s| s.as_str())
                  .unwrap_or("unknown");

                if should_log(stats.total_received) || !sequence_ok {
                  info!(
                    "Received object {}: track={} group={}, object={}, seq={}",
                    stats.total_received,
                    label,
                    obj.location.group,
                    obj.location.object,
                    if sequence_ok { "OK" } else { "GAP" }
                  );
                } else {
                  debug!(
                    "Received object {}: track={} group={}, object={}",
                    stats.total_received, label, obj.location.group, obj.location.object
                  );
                }
                handler = next_handler;
              }
              None => {
                debug!("Stream closed");
                break;
              }
            }
          }
        }
        Err(e) => {
          info!("Stream accept ended: {:?}", e);
          break;
        }
      }
    }

    stats.report();
    stats
  });

  if duration > 0 {
    tokio::time::sleep(tokio::time::Duration::from_secs(duration)).await;
    info!("Duration elapsed, closing connection...");
    connection.close(0u32, b"Done");
  }

  let stats = stream_task.await?;
  info!(
    "Subscriber finished: received={}, errors={}, gaps={}",
    stats.total_received, stats.parse_errors, stats.sequence_gaps
  );

  Ok(())
}
