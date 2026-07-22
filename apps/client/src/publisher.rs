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
use crate::utils::should_log;
use anyhow::Result;
use bytes::Bytes;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish::Publish;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::request_ok::RequestOk;
use moqtail::model::control::subscribe_ok::SubscribeOk;
use moqtail::model::data::datagram::Datagram;
use moqtail::model::data::object::Object;
use moqtail::model::data::subgroup_header::SubgroupHeader;
use moqtail::model::data::subgroup_object::SubgroupObject;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::model::property::object_property::ObjectProperty;
use moqtail::transport::connection::TransportConnection;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{HeaderInfo, SendDataStream};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

pub struct PublishConfig {
  pub namespace: String,
  pub track_name: String,
  pub delivery_mode: DeliveryMode,
  pub group_count: u64,
  pub interval: u64,
  pub objects_per_group: u64,
  pub object_id_step: u64,
  pub group_id_step: u64,
  pub payload_size: usize,
  pub track_alias: u64,
  pub publisher_priority: u8,
}

pub struct PublishNamespaceConfig {
  pub namespace: String,
  pub delivery_mode: DeliveryMode,
  pub group_count: u64,
  pub interval: u64,
  pub objects_per_group: u64,
  pub object_id_step: u64,
  pub group_id_step: u64,
  pub payload_size: usize,
  pub publisher_priority: u8,
}

pub async fn run_namespace(moq: MoqConnection, config: PublishNamespaceConfig) -> Result<()> {
  // Keep `moq` alive for the session; the control stream carries only SETUP now.
  let connection = moq.connection.clone();

  let ns = Tuple::from_utf8_path(&config.namespace);

  // Step 1: Announce namespace on its own request stream (kept open below).
  let _namespace_stream = publish_namespace(&connection, &ns).await?;

  let data_config = DataConfig {
    delivery_mode: config.delivery_mode,
    group_count: config.group_count,
    interval: config.interval,
    objects_per_group: config.objects_per_group,
    object_id_step: config.object_id_step,
    group_id_step: config.group_id_step,
    payload_size: config.payload_size,
    publisher_priority: config.publisher_priority,
  };

  // Step 2: the relay forwards each SUBSCRIBE on its own bidirectional request
  // stream. Accept those streams and answer SUBSCRIBE on the same stream.
  let track_alias_counter = Arc::new(std::sync::atomic::AtomicU64::new(1));

  info!(
    "Waiting for Subscribe request streams on namespace '{}'...",
    config.namespace
  );

  loop {
    let (send, recv) = match connection.accept_bi().await {
      Ok(streams) => streams,
      Err(e) => {
        info!("Request stream accept ended: {:?}", e);
        break;
      }
    };

    let conn = connection.clone();
    let dc = data_config.clone();
    let counter = track_alias_counter.clone();
    tokio::spawn(async move {
      let mut request_stream = ControlStreamHandler::new(send, recv);
      match request_stream.next_message().await {
        Ok(ControlMessage::Subscribe(m)) => {
          let track_alias = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
          info!(
            "Received Subscribe on request stream: request_id={}, track={:?}, assigning track_alias={}",
            m.request_id, m.track_name, track_alias
          );

          let ok = SubscribeOk::new(track_alias, vec![], vec![]);
          if let Err(e) = request_stream.send_impl(&ok).await {
            error!("Failed to send SubscribeOk: {:?}", e);
            return;
          }
          info!(
            "SubscribeOk sent for request_id={}, track_alias={}",
            m.request_id, track_alias
          );

          // Serve data, but stop if the subscriber cancels by resetting the stream.
          tokio::select! {
            res = send_data(&conn, track_alias, &dc) => {
              if let Err(e) = res {
                error!("Data sending failed: {:?}", e);
              }
            }
            _ = request_stream.next_message() => {
              info!("Subscriber cancelled (stream reset); stopping data delivery");
            }
          }
          drop(request_stream);
        }
        Ok(ControlMessage::TrackStatus(m)) => {
          info!(
            "Received TrackStatus on request stream for track={:?}",
            m.track_name
          );
          let ok = RequestOk::new(vec![]);
          if let Err(e) = request_stream.send_impl(&ok).await {
            error!("Failed to send TrackStatus RequestOk: {:?}", e);
          }
          drop(request_stream);
        }
        Ok(ControlMessage::Publish(m)) => {
          info!(
            "Received pushed Publish on request stream for track={:?}",
            m.track_name
          );
          let ok = RequestOk::new(vec![]);
          if let Err(e) = request_stream.send_impl(&ok).await {
            error!("Failed to send PublishOk: {:?}", e);
          }
          drop(request_stream);
        }
        Ok(other) => info!("Unexpected message on request stream: {:?}", other),
        Err(e) => info!("Request stream read error: {:?}", e),
      }
    });
  }

  // Keep connection alive briefly to ensure delivery
  info!("Waiting before closing connection...");
  tokio::time::sleep(Duration::from_secs(2)).await;

  info!("Closing connection...");
  connection.close(0u32, b"Done");

  Ok(())
}

pub async fn run(moq: MoqConnection, config: PublishConfig) -> Result<()> {
  // Keep `moq` alive for the whole function: its control stream carries only
  // SETUP now, but must stay open for the session's lifetime. The PUBLISH request
  // uses its own bidi stream; objects go out on uni streams.
  let connection = moq.connection.clone();

  let ns = Tuple::from_utf8_path(&config.namespace);

  let data_config = DataConfig {
    delivery_mode: config.delivery_mode,
    group_count: config.group_count,
    interval: config.interval,
    objects_per_group: config.objects_per_group,
    object_id_step: config.object_id_step,
    group_id_step: config.group_id_step,
    payload_size: config.payload_size,
    publisher_priority: config.publisher_priority,
  };

  publish_track(
    &connection,
    &ns,
    &config.track_name,
    config.track_alias,
    &data_config,
  )
  .await?;

  // Keep connection alive briefly to ensure delivery
  info!("Waiting before closing connection...");
  tokio::time::sleep(Duration::from_secs(2)).await;

  info!("Closing connection...");
  connection.close(0u32, b"Done");

  Ok(())
}

/// Announce a namespace on its own bidi request stream. Returns the stream, which
/// the caller keeps open for the announcement's lifetime.
async fn publish_namespace(
  connection: &Arc<TransportConnection>,
  namespace: &Tuple,
) -> Result<ControlStreamHandler> {
  info!("Publishing namespace...");
  let publish_namespace = PublishNamespace::new(0, namespace.clone(), &[]);

  let (send, recv) = connection.open_bi().await?;
  let mut request_stream = ControlStreamHandler::new(send, recv);
  request_stream
    .send(&ControlMessage::PublishNamespace(Box::new(
      publish_namespace,
    )))
    .await
    .map_err(|e| anyhow::anyhow!("Failed to send PUBLISH_NAMESPACE: {:?}", e))?;

  // The response returns on this same request stream, so it needs no request id.
  match request_stream.next_message().await {
    Ok(ControlMessage::RequestOk(_)) => {
      info!("Namespace published successfully");
      Ok(request_stream)
    }
    Ok(m) => anyhow::bail!("Expected RequestOk, got {:?}", m),
    Err(e) => anyhow::bail!("Failed waiting for RequestOk: {:?}", e),
  }
}

#[derive(Clone)]
struct DataConfig {
  delivery_mode: DeliveryMode,
  group_count: u64,
  interval: u64,
  objects_per_group: u64,
  object_id_step: u64,
  group_id_step: u64,
  payload_size: usize,
  publisher_priority: u8,
}

async fn publish_track(
  connection: &Arc<TransportConnection>,
  namespace: &Tuple,
  track_name: &str,
  track_alias: u64,
  data_config: &DataConfig,
) -> Result<()> {
  info!("Publishing track: track_alias={}", track_alias);
  let publish = Publish::new(
    0, // request_id
    namespace.clone(),
    TupleField::from_utf8(track_name),
    track_alias,
    vec![MessageParameter::Forward { forward: true }],
    vec![],
  );

  // PUBLISH opens its own bidi request stream; the response returns on it.
  let (send, recv) = connection.open_bi().await?;
  let mut request_stream = ControlStreamHandler::new(send, recv);
  request_stream
    .send(&ControlMessage::Publish(Box::new(publish)))
    .await
    .map_err(|e| anyhow::anyhow!("Failed to send PUBLISH: {:?}", e))?;

  match request_stream.next_message().await {
    // PUBLISH is answered by REQUEST_OK (PUBLISH_OK); Track Properties must be empty.
    Ok(ControlMessage::RequestOk(m)) => {
      m.validate_track_properties(false)
        .map_err(|_| anyhow::anyhow!("PUBLISH_OK carried Track Properties"))?;
      info!("Track published");
    }
    Ok(m) => anyhow::bail!("Expected REQUEST_OK, got {:?}", m),
    Err(e) => anyhow::bail!("Failed waiting for REQUEST_OK: {:?}", e),
  }

  // Hold the request stream open while objects are delivered on uni streams.
  let result = send_data(connection, track_alias, data_config).await;
  drop(request_stream);
  result
}

async fn send_data(
  connection: &Arc<TransportConnection>,
  track_alias: u64,
  config: &DataConfig,
) -> Result<()> {
  match config.delivery_mode {
    DeliveryMode::Datagram => {
      send_datagrams(
        connection,
        track_alias,
        config.group_count,
        config.interval,
        config.objects_per_group,
        config.object_id_step,
        config.group_id_step,
        config.payload_size,
        config.publisher_priority,
      )
      .await
    }
    DeliveryMode::Subgroup => {
      send_via_streams(
        connection,
        track_alias,
        config.group_count,
        config.interval,
        config.objects_per_group,
        config.object_id_step,
        config.group_id_step,
        config.payload_size,
        config.publisher_priority,
      )
      .await
    }
  }
}

#[allow(clippy::too_many_arguments)]
async fn send_datagrams(
  connection: &TransportConnection,
  track_alias: u64,
  group_count: u64,
  interval_ms: u64,
  objects_per_group: u64,
  object_id_step: u64,
  group_id_step: u64,
  payload_size: usize,
  publisher_priority: u8,
) -> Result<()> {
  let interval = Duration::from_millis(interval_ms);
  info!(
    "Sending datagrams: {} groups, {} objects/group, object-id step {}, group-id step {}, {} byte payloads",
    group_count, objects_per_group, object_id_step, group_id_step, payload_size
  );

  for g in 0..group_count {
    let group_id = g * group_id_step;
    for i in 0..objects_per_group {
      let object_id = i * object_id_step;
      let payload = generate_payload(payload_size);

      // Communicate the ID gaps explicitly (draft 12.8 / 12.9).
      let mut properties = Vec::new();
      if g > 0 && i == 0 && group_id_step > 1 {
        properties.push(ObjectProperty::PriorGroupIdGap {
          gap: group_id_step - 1,
        });
      }
      if i > 0 && object_id_step > 1 {
        properties.push(ObjectProperty::PriorObjectIdGap {
          gap: object_id_step - 1,
        });
      }

      let datagram_obj = Datagram::new_payload(
        track_alias,
        group_id,
        object_id,
        Some(publisher_priority),
        Some(properties),
        Bytes::from(payload),
        false, // end_of_group
      );

      let serialized = datagram_obj.serialize()?;

      match connection.send_datagram(serialized) {
        Ok(_) => {
          let total = g * objects_per_group + i;
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

#[allow(clippy::too_many_arguments)]
async fn send_via_streams(
  connection: &TransportConnection,
  track_alias: u64,
  group_count: u64,
  interval_ms: u64,
  objects_per_group: u64,
  object_id_step: u64,
  group_id_step: u64,
  payload_size: usize,
  publisher_priority: u8,
) -> Result<()> {
  let interval = Duration::from_millis(interval_ms);
  info!(
    "Sending via streams: {} groups, {} objects/group, object-id step {}, group-id step {}, {} byte payloads",
    group_count, objects_per_group, object_id_step, group_id_step, payload_size
  );

  for g in 0..group_count {
    let group_id = g * group_id_step;
    info!("Opening stream for group {}", group_id);
    let stream = connection.open_uni().await?;

    let sub_header = SubgroupHeader::new_with_explicit_id(
      track_alias,
      group_id,
      1u64,
      Some(publisher_priority),
      true,
      true,
      true, // first_object: a fresh stream is opened per group, so its first object is
            // the first the publisher produced in this subgroup
    );
    let header_info = HeaderInfo::Subgroup { header: sub_header };
    let stream = Arc::new(Mutex::new(stream));
    let mut handler = SendDataStream::new(stream, header_info).await?;

    let mut prev_object_id = None;
    for i in 0..objects_per_group {
      let object_id = i * object_id_step;
      let payload = generate_payload(payload_size);

      // Communicate the ID gaps explicitly (draft 12.8 / 12.9): the first object
      // of a skipped group carries Prior Group ID Gap; every skipped object
      // carries Prior Object ID Gap.
      let mut properties = Vec::new();
      if g > 0 && i == 0 && group_id_step > 1 {
        properties.push(ObjectProperty::PriorGroupIdGap {
          gap: group_id_step - 1,
        });
      }
      if i > 0 && object_id_step > 1 {
        properties.push(ObjectProperty::PriorObjectIdGap {
          gap: object_id_step - 1,
        });
      }

      let subgroup_obj = SubgroupObject {
        object_id,
        properties: Some(properties),
        object_status: None,
        payload: Some(Bytes::from(payload)),
      };
      let object =
        Object::try_from_subgroup(subgroup_obj, track_alias, group_id, Some(group_id), Some(1))?;

      match handler.send_object(&object, prev_object_id).await {
        Ok(_) => {
          let total = g * objects_per_group + i;
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
