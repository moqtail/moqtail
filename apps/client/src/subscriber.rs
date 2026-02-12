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
use moqtail::model::data::datagram_object::DatagramObject;
use moqtail::transport::data_stream_handler::RecvDataStream;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

pub async fn run(
  moq: MoqConnection,
  namespace: &str,
  track_name: &str,
  forwarding_preference: ForwardingPreference,
  duration: u64,
) -> Result<()> {
  let MoqConnection {
    connection,
    mut control_stream,
  } = moq;

  // Subscribe to track
  let ns = Tuple::from_utf8_path(namespace);
  info!("Subscribing to track: {}/{}", namespace, track_name);
  let subscribe = Subscribe::new_latest_object(
    0, // request_id
    ns,
    TupleField::from_utf8(track_name), // track_name
    0,                                 // subscriber_priority
    GroupOrder::Ascending,
    true, // forward
    vec![],
  );
  control_stream
    .send(&ControlMessage::Subscribe(Box::new(subscribe)))
    .await?;

  // Wait for SubscribeOk
  info!("Waiting for SubscribeOk...");
  let track_alias = match control_stream.next_message().await {
    Ok(ControlMessage::SubscribeOk(m)) => {
      info!(
        "Subscribed: track_alias={}, content_exists={}",
        m.track_alias, m.content_exists
      );
      m.track_alias
    }
    Ok(m) => anyhow::bail!("Expected SubscribeOk, got {:?}", m),
    Err(e) => anyhow::bail!("Failed waiting for SubscribeOk: {:?}", e),
  };

  match forwarding_preference {
    ForwardingPreference::Datagram => receive_datagrams(&connection, track_alias, duration).await,
    ForwardingPreference::Subgroup => receive_streams(&connection, track_alias, duration).await,
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

          match DatagramObject::deserialize(&mut bytes_mut) {
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
                  obj.payload.len(),
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
  _track_alias: u64,
  duration: u64,
) -> Result<()> {
  info!("Listening for incoming streams...");

  let pending_fetches = Arc::new(RwLock::new(BTreeMap::new()));
  let conn = connection.clone();
  let pending_fetches_clone = pending_fetches.clone();

  let stream_task = tokio::spawn(async move {
    let mut stats = ReceptionStats::new();

    loop {
      match conn.accept_uni().await {
        Ok(stream) => {
          info!("Accepted unidirectional stream");
          let stream_handler = RecvDataStream::new(stream, pending_fetches_clone.clone());
          let mut handler = &stream_handler;

          loop {
            let (next_handler, object) = handler.next_object().await;
            match object {
              Some(obj) => {
                let sequence_ok = stats.record_object(obj.location.group, obj.location.object);

                if should_log(stats.total_received) || !sequence_ok {
                  info!(
                    "Received object {}: group={}, object={}, seq={}",
                    stats.total_received,
                    obj.location.group,
                    obj.location.object,
                    if sequence_ok { "OK" } else { "GAP" }
                  );
                } else {
                  debug!(
                    "Received object {}: group={}, object={}",
                    stats.total_received, obj.location.group, obj.location.object
                  );
                }
                handler = next_handler;
              }
              None => {
                info!("Stream closed");
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
