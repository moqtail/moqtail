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

use crate::cli::{ForwardingPreference, PublishMode};
use crate::connection::MoqConnection;
use crate::utils::should_log;
use anyhow::Result;
use bytes::Bytes;
use moqtail::model::common::location::Location;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish::Publish;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::subscribe_ok::SubscribeOk;
use moqtail::model::data::datagram_object::DatagramObject;
use moqtail::model::data::object::Object;
use moqtail::model::data::subgroup_header::SubgroupHeader;
use moqtail::model::data::subgroup_object::SubgroupObject;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{HeaderInfo, SendDataStream};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

pub struct PublishConfig {
  pub namespace: String,
  pub track_name: String,
  pub forwarding_preference: ForwardingPreference,
  pub publish_mode: PublishMode,
  pub group_count: u64,
  pub interval: u64,
  pub objects_per_group: u64,
  pub payload_size: usize,
  pub track_alias: u64,
}

pub async fn run(moq: MoqConnection, config: PublishConfig) -> Result<()> {
  let MoqConnection {
    connection,
    mut control_stream,
  } = moq;

  let ns = Tuple::from_utf8_path(&config.namespace);

  // Step 1: PublishNamespace
  if config.publish_mode == PublishMode::Proactive {
    info!("Publishing namespace proactively...");
  } else {
    info!("Publishing namespace reactively...");
    publish_namespace(&mut control_stream, &ns).await?;
  }

  let data_config = DataConfig {
    forwarding_preference: config.forwarding_preference,
    group_count: config.group_count,
    interval: config.interval,
    objects_per_group: config.objects_per_group,
    payload_size: config.payload_size,
  };

  // Step 2: Dispatch based on publish mode
  match config.publish_mode {
    PublishMode::Proactive => {
      publish_track_proactive(
        &connection,
        &mut control_stream,
        &ns,
        &config.track_name,
        config.track_alias,
        &data_config,
      )
      .await?;
    }
    PublishMode::Reactive => {
      publish_track_reactive(
        &connection,
        &mut control_stream,
        config.track_alias,
        &data_config,
      )
      .await?;
    }
  }

  // Keep connection alive briefly to ensure delivery
  info!("Waiting before closing connection...");
  tokio::time::sleep(Duration::from_secs(2)).await;

  info!("Closing connection...");
  connection.close(0u32.into(), b"Done");

  Ok(())
}

async fn publish_namespace(
  control_stream: &mut ControlStreamHandler,
  namespace: &Tuple,
) -> Result<()> {
  info!("Publishing namespace...");
  let publish_namespace = PublishNamespace::new(0, namespace.clone(), &[]);
  control_stream
    .send(&ControlMessage::PublishNamespace(Box::new(
      publish_namespace,
    )))
    .await?;

  match control_stream.next_message().await {
    Ok(ControlMessage::PublishNamespaceOk(_)) => {
      info!("Namespace published successfully");
      Ok(())
    }
    Ok(m) => anyhow::bail!("Expected PublishNamespaceOk, got {:?}", m),
    Err(e) => anyhow::bail!("Failed waiting for PublishNamespaceOk: {:?}", e),
  }
}

struct DataConfig {
  forwarding_preference: ForwardingPreference,
  group_count: u64,
  interval: u64,
  objects_per_group: u64,
  payload_size: usize,
}

/// Proactive mode: send Publish message, get PublishOk, then send data immediately.
async fn publish_track_proactive(
  connection: &Arc<wtransport::Connection>,
  control_stream: &mut ControlStreamHandler,
  namespace: &Tuple,
  track_name: &str,
  track_alias: u64,
  data_config: &DataConfig,
) -> Result<()> {
  info!(
    "Publishing track (proactive mode): track_alias={}",
    track_alias
  );
  let publish = Publish::new(
    1, // request_id
    namespace.clone(),
    TupleField::from_utf8(track_name),
    track_alias,
    GroupOrder::Ascending,
    1, // content_exists
    Some(Location::new(0, 0)),
    1, // forward
    vec![],
  );
  control_stream
    .send(&ControlMessage::Publish(Box::new(publish)))
    .await?;

  match control_stream.next_message().await {
    Ok(ControlMessage::PublishOk(m)) => {
      info!("Track published, request_id: {}", m.request_id);
    }
    Ok(m) => anyhow::bail!("Expected PublishOk, got {:?}", m),
    Err(e) => anyhow::bail!("Failed waiting for PublishOk: {:?}", e),
  }

  send_data(connection, track_alias, data_config).await
}

/// Reactive mode: wait for Subscribe from relay, send SubscribeOk, then publish.
async fn publish_track_reactive(
  connection: &Arc<wtransport::Connection>,
  control_stream: &mut ControlStreamHandler,
  track_alias: u64,
  data_config: &DataConfig,
) -> Result<()> {
  info!("Waiting for Subscribe from relay (reactive mode)...");

  loop {
    match control_stream.next_message().await {
      Ok(ControlMessage::Subscribe(m)) => {
        info!("Received Subscribe: {:?}", m);
        let msg = SubscribeOk::new_ascending_with_content(m.request_id, track_alias, 0, None, None);
        control_stream.send_impl(&msg).await?;
        info!("SubscribeOk sent, track_alias={}", track_alias);

        send_data(connection, track_alias, data_config).await?;
        return Ok(());
      }
      Ok(ControlMessage::Unsubscribe(m)) => {
        info!("Received Unsubscribe: {:?}", m);
        return Ok(());
      }
      Ok(m) => {
        info!(
          "Received control message while waiting for Subscribe: {:?}",
          m
        );
      }
      Err(e) => {
        error!("Error receiving control message: {:?}", e);
        anyhow::bail!("Error waiting for Subscribe: {:?}", e);
      }
    }
  }
}

async fn send_data(
  connection: &Arc<wtransport::Connection>,
  track_alias: u64,
  config: &DataConfig,
) -> Result<()> {
  match config.forwarding_preference {
    ForwardingPreference::Datagram => {
      send_datagrams(
        connection,
        track_alias,
        config.group_count,
        config.interval,
        config.objects_per_group,
        config.payload_size,
      )
      .await
    }
    ForwardingPreference::Subgroup => {
      send_via_streams(
        connection,
        track_alias,
        config.group_count,
        config.interval,
        config.objects_per_group,
        config.payload_size,
      )
      .await
    }
  }
}

async fn send_datagrams(
  connection: &wtransport::Connection,
  track_alias: u64,
  group_count: u64,
  interval_ms: u64,
  objects_per_group: u64,
  payload_size: usize,
) -> Result<()> {
  let interval = Duration::from_millis(interval_ms);
  info!(
    "Sending datagrams: {} groups, {} objects/group, {} byte payloads",
    group_count, objects_per_group, payload_size
  );

  for group_id in 0..group_count {
    for object_id in 0..objects_per_group {
      let payload = generate_payload(payload_size);

      let datagram_obj = DatagramObject::new(
        track_alias,
        group_id,
        object_id,
        0, // publisher_priority
        Bytes::from(payload),
      );

      let serialized = datagram_obj.serialize()?;

      match connection.send_datagram(serialized) {
        Ok(_) => {
          let total = group_id * objects_per_group + object_id;
          if should_log(total) {
            info!(
              "Sent datagram: group={}, object={}, size={} bytes",
              group_id, object_id, payload_size
            );
          } else {
            debug!("Sent datagram: group={}, object={}", group_id, object_id);
          }
        }
        Err(e) => {
          error!(
            "Failed to send datagram: group={}, object={}, error={:?}",
            group_id, object_id, e
          );
        }
      }

      tokio::time::sleep(interval).await;
    }
  }

  info!("All datagrams sent");
  Ok(())
}

async fn send_via_streams(
  connection: &wtransport::Connection,
  track_alias: u64,
  group_count: u64,
  interval_ms: u64,
  objects_per_group: u64,
  payload_size: usize,
) -> Result<()> {
  let interval = Duration::from_millis(interval_ms);
  info!(
    "Sending via streams: {} groups, {} objects/group, {} byte payloads",
    group_count, objects_per_group, payload_size
  );

  for group_id in 0..group_count {
    info!("Opening stream for group {}", group_id);
    let stream = connection.open_uni().await?.await?;

    let sub_header =
      SubgroupHeader::new_with_explicit_id(track_alias, group_id, 1u64, 1u8, true, true);
    let header_info = HeaderInfo::Subgroup { header: sub_header };
    let stream = Arc::new(Mutex::new(stream));
    let mut handler = SendDataStream::new(stream, header_info).await?;

    let mut prev_object_id = None;
    for object_id in 0..objects_per_group {
      let payload = generate_payload(payload_size);

      let subgroup_obj = SubgroupObject {
        object_id,
        extension_headers: Some(vec![]),
        object_status: None,
        payload: Some(Bytes::from(payload)),
      };
      let object =
        Object::try_from_subgroup(subgroup_obj, track_alias, group_id, Some(group_id), 1)?;

      match handler.send_object(&object, prev_object_id).await {
        Ok(_) => {
          let total = group_id * objects_per_group + object_id;
          if should_log(total) {
            info!(
              "Sent object: group={}, object={}, size={} bytes",
              group_id, object_id, payload_size
            );
          } else {
            debug!("Sent object: group={}, object={}", group_id, object_id);
          }
        }
        Err(e) => {
          error!(
            "Failed to send object: group={}, object={}, error={:?}",
            group_id, object_id, e
          );
        }
      }
      prev_object_id = Some(object_id);
      tokio::time::sleep(interval).await;
    }

    handler.flush().await?;
    info!("Stream flushed for group {}", group_id);
  }

  info!("All streams sent");
  Ok(())
}

fn generate_payload(size: usize) -> Vec<u8> {
  // Simple PRNG for reproducible test payloads
  let mut seed: u64 = 0x123456789abcdef0;
  (0..size)
    .map(|_| {
      seed ^= seed << 13;
      seed ^= seed >> 7;
      seed ^= seed << 17;
      (seed & 0xFF) as u8
    })
    .collect()
}
