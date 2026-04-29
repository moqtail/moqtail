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

use crate::cli::ForwardingPreference;
use crate::connection::MoqConnection;
use crate::stats::ReceptionStats;
use crate::utils::should_log;
use anyhow::Result;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::datagram::Datagram;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::transport::data_stream_handler::RecvDataStream;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// Subscribe to one track and return its assigned track alias.
async fn subscribe_track(
  control_stream: &mut moqtail::transport::control_stream_handler::ControlStreamHandler,
  namespace: &str,
  track_name: &str,
  request_id: u64,
  subscriber_priority: u8,
  group_order: GroupOrder,
) -> Result<u64> {
  let ns = Tuple::from_utf8_path(namespace);
  info!(
    "Subscribing to track: {}/{} (request_id={}, priority={})",
    namespace, track_name, request_id, subscriber_priority
  );
  let subscribe = Subscribe::new_latest_object(
    request_id,
    ns,
    TupleField::from_utf8(track_name),
    vec![
      MessageParameter::new_subscriber_priority(subscriber_priority),
      MessageParameter::new_group_order(group_order),
      MessageParameter::new_forward(true),
    ],
  );
  control_stream
    .send(&ControlMessage::Subscribe(Box::new(subscribe)))
    .await?;

  match control_stream.next_message().await {
    Ok(ControlMessage::SubscribeOk(m)) => {
      info!(
        "Subscribed: track={} track_alias={}",
        track_name, m.track_alias
      );
      Ok(m.track_alias)
    }
    Ok(m) => anyhow::bail!("Expected SubscribeOk for {}, got {:?}", track_name, m),
    Err(e) => anyhow::bail!("Failed waiting for SubscribeOk for {}: {:?}", track_name, e),
  }
}

pub async fn run(
  moq: MoqConnection,
  namespace: &str,
  track_name: &str,
  forwarding_preference: ForwardingPreference,
  duration: u64,
  subscriber_priority: u8,
  group_order: GroupOrder,
  // Optional second track for priority testing: (track_name, subscriber_priority)
  extra_track: Option<(String, u8)>,
) -> Result<()> {
  let MoqConnection {
    connection,
    mut control_stream,
  } = moq;

  let track_alias = subscribe_track(
    &mut control_stream,
    namespace,
    track_name,
    0,
    subscriber_priority,
    group_order,
  )
  .await?;

  let extra_alias = if let Some((ref extra_name, extra_priority)) = extra_track {
    let alias = subscribe_track(
      &mut control_stream,
      namespace,
      extra_name,
      1,
      extra_priority,
      group_order,
    )
    .await?;
    Some((extra_name.clone(), alias))
  } else {
    None
  };

  match forwarding_preference {
    ForwardingPreference::Datagram => receive_datagrams(&connection, track_alias, duration).await,
    ForwardingPreference::Subgroup => {
      receive_streams(&connection, track_alias, extra_alias, duration).await
    }
  }
}

async fn receive_datagrams(
  connection: &Arc<wtransport::Connection>,
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
          let bytes = bytes::Bytes::from(datagram.payload().to_vec());
          let mut bytes_mut = bytes.clone();

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
    connection.close(0u32.into(), b"Done");
  }

  let stats = datagram_task.await?;
  info!(
    "Subscriber finished: received={}, errors={}, gaps={}",
    stats.total_received, stats.parse_errors, stats.sequence_gaps
  );

  Ok(())
}

async fn receive_streams(
  connection: &Arc<wtransport::Connection>,
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
    connection.close(0u32.into(), b"Done");
  }

  let stats = stream_task.await?;
  info!(
    "Subscriber finished: received={}, errors={}, gaps={}",
    stats.total_received, stats.parse_errors, stats.sequence_gaps
  );

  Ok(())
}
