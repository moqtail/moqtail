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
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::Fetch;
use moqtail::model::control::fetch_ok::FetchOk;
use moqtail::model::control::publish::Publish;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::request_ok::RequestOk;
use moqtail::model::control::subscribe_ok::SubscribeOk;
use moqtail::model::data::datagram::Datagram;
use moqtail::model::data::fetch_header::FetchHeader;
use moqtail::model::data::object::Object;
use moqtail::model::data::subgroup_header::SubgroupHeader;
use moqtail::model::data::subgroup_object::SubgroupObject;
use moqtail::model::error::{RequestErrorCode, StreamResetCode};
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
  pub withdraw_after: u64,
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

  // Step 1: Announce namespace on its own request stream. The namespace is published
  // for exactly as long as that stream is open, so a publisher withdraws by resetting
  // it -- and keeps the announcement by simply holding it.
  let namespace_stream = publish_namespace(&connection, &ns).await?;
  let _namespace_stream = if config.withdraw_after > 0 {
    let after = config.withdraw_after;
    let namespace = config.namespace.clone();
    tokio::spawn(async move {
      tokio::time::sleep(Duration::from_secs(after)).await;
      info!("Withdrawing namespace '{namespace}' after {after}s; session stays open");
      namespace_stream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
    });
    None
  } else {
    Some(namespace_stream)
  };

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

  // Step 2: the relay sends each request on its own bidirectional stream.
  let published = Published::default();

  info!(
    "Waiting for request streams on namespace '{}'...",
    config.namespace
  );

  serve_request_streams(connection.clone(), data_config, published).await;

  // Keep connection alive briefly to ensure delivery
  info!("Waiting before closing connection...");
  tokio::time::sleep(Duration::from_secs(2)).await;

  info!("Closing connection...");
  connection.close(0u32, b"Done");

  Ok(())
}

/// Accepts request streams and answers what a publisher owes on them: SUBSCRIBE with
/// objects, FETCH from what has already been published, TRACK_STATUS and pushed PUBLISH
/// with an acknowledgement.
async fn serve_request_streams(
  connection: Arc<TransportConnection>,
  data_config: DataConfig,
  published: Published,
) {
  let track_alias_counter = Arc::new(std::sync::atomic::AtomicU64::new(1));

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
    let pub_state = published.clone();
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
            res = send_data(&conn, track_alias, &dc, &pub_state) => {
              match res {
                Ok(()) => {
                  // Finished delivering; signal completion on the request stream.
                  send_publish_done(&mut request_stream, m.request_id, stream_count(&dc)).await;
                }
                Err(e) => error!("Data sending failed: {:?}", e),
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
        Ok(ControlMessage::Fetch(m)) => {
          serve_fetch(&conn, &mut request_stream, *m, &dc, &pub_state).await;
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

  // A relay may FETCH a track it was pushed, so the request streams are served
  // alongside delivery rather than after it.
  let published = Published::default();
  let serving = tokio::spawn(serve_request_streams(
    connection.clone(),
    data_config.clone(),
    published.clone(),
  ));

  publish_track(
    &connection,
    &ns,
    &config.track_name,
    config.track_alias,
    &data_config,
    &published,
  )
  .await?;
  serving.abort();

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

/// The largest Location actually sent so far, shared with whatever is answering a
/// FETCH. Delivery is paced, so what has been published trails what is configured, and
/// a FETCH must be answered from the former.
#[derive(Clone, Default)]
struct Published(Arc<Mutex<Option<Location>>>);

impl Published {
  async fn record(&self, group_id: u64, object_id: u64) {
    let location = Location::new(group_id, object_id);
    let mut largest = self.0.lock().await;
    if largest.as_ref().is_none_or(|current| *current < location) {
      *largest = Some(location);
    }
  }

  async fn largest(&self) -> Option<Location> {
    self.0.lock().await.clone()
  }
}

/// Answers a standalone FETCH from what has already been published.
///
/// The End Location is the requested one unless it runs past the last Object sent, in
/// which case it is clamped to that -- a publisher answers for what exists, not for what
/// it intends to send later. Objects then go out on their own unidirectional stream,
/// which is closed when the range is done.
async fn serve_fetch(
  connection: &Arc<TransportConnection>,
  request_stream: &mut ControlStreamHandler,
  fetch: Fetch,
  config: &DataConfig,
  published: &Published,
) {
  let Some(largest) = published.largest().await else {
    info!(
      "FETCH for {:?} before anything was published",
      fetch.track_name
    );
    reject_fetch(
      request_stream,
      RequestErrorCode::InvalidRange,
      "nothing published yet",
    )
    .await;
    return;
  };

  if fetch.start_location > largest {
    info!(
      "FETCH start {:?} is past the last published Object {:?}",
      fetch.start_location, largest
    );
    reject_fetch(
      request_stream,
      RequestErrorCode::InvalidRange,
      "start beyond published data",
    )
    .await;
    return;
  }

  // End Location is the last Object plus one, and 0 in the Object field means the whole
  // group.
  let requested_end = if fetch.end_location.object == 0 {
    Location::new(fetch.end_location.group, u64::MAX)
  } else {
    fetch.end_location.clone()
  };
  let available_end = Location::new(largest.group, largest.object + 1);
  let end_location = if requested_end > available_end {
    available_end
  } else {
    fetch.end_location.clone()
  };

  info!(
    "Serving FETCH {} for {:?}: requested up to {:?}, answering up to {:?}",
    fetch.request_id, fetch.track_name, fetch.end_location, end_location
  );

  let ok = FetchOk::new(false, end_location.clone(), vec![], vec![]);
  if let Err(e) = request_stream.send_impl(&ok).await {
    error!("Failed to send FetchOk: {:?}", e);
    return;
  }

  if let Err(e) = send_fetch_objects(
    connection,
    &fetch,
    &fetch.start_location,
    &end_location,
    config,
  )
  .await
  {
    error!("Failed to serve FETCH objects: {:?}", e);
  }
}

async fn reject_fetch(
  request_stream: &mut ControlStreamHandler,
  code: RequestErrorCode,
  reason: &str,
) {
  let reason = ReasonPhrase::try_new(reason.to_string()).expect("reason within length limit");
  let err = RequestError::new(code, 0, reason);
  if let Err(e) = request_stream.send_impl(&err).await {
    error!("Failed to send FETCH RequestError: {:?}", e);
  }
}

/// Regenerates the Objects in `[start, end)` onto a fetch stream. The payloads are
/// synthetic and reproducible, so a fetch returns the same bytes the live delivery did.
async fn send_fetch_objects(
  connection: &Arc<TransportConnection>,
  fetch: &Fetch,
  start: &Location,
  end: &Location,
  config: &DataConfig,
) -> Result<()> {
  let stream = connection.open_uni().await?;
  let header_info = HeaderInfo::Fetch {
    header: FetchHeader::new(fetch.request_id),
    fetch_request: Some(fetch.clone()),
  };
  let stream = Arc::new(Mutex::new(stream));
  let mut handler = SendDataStream::new(stream.clone(), header_info).await?;

  let mut sent = 0u64;
  for g in 0..config.group_count {
    let group_id = g * config.group_id_step;
    if group_id < start.group || group_id > end.group {
      continue;
    }
    for i in 0..config.objects_per_group {
      let object_id = i * config.object_id_step;
      let location = Location::new(group_id, object_id);
      if location < *start || location >= *end {
        continue;
      }

      let subgroup_obj = SubgroupObject {
        object_id,
        properties: None,
        object_status: None,
        payload: Some(Bytes::from(generate_payload(config.payload_size))),
      };
      let object = Object::try_from_subgroup(
        subgroup_obj,
        fetch.request_id,
        group_id,
        Some(group_id),
        Some(config.publisher_priority),
      )?;
      handler.send_object(&object, None).await?;
      sent += 1;
    }
  }

  stream.lock().await.finish().await?;
  info!("Served {} objects for FETCH {}", sent, fetch.request_id);
  Ok(())
}

/// Send PUBLISH_DONE as the final message on a subscription's request stream to
/// signal the publisher is done sending objects for it.
async fn send_publish_done(
  request_stream: &mut ControlStreamHandler,
  request_id: u64,
  stream_count: u64,
) {
  let reason = ReasonPhrase::try_new("done".to_string()).expect("reason within length limit");
  let done = PublishDone::new(PublishDoneStatusCode::TrackEnded, stream_count, reason);
  if let Err(e) = request_stream
    .send(&ControlMessage::PublishDone(Box::new(done)))
    .await
  {
    error!("Failed to send PUBLISH_DONE: {:?}", e);
  } else {
    info!("Sent PUBLISH_DONE for request_id={}", request_id);
  }
}

async fn publish_track(
  connection: &Arc<TransportConnection>,
  namespace: &Tuple,
  track_name: &str,
  track_alias: u64,
  data_config: &DataConfig,
  published: &Published,
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
  let result = send_data(connection, track_alias, data_config, published).await;

  // PUBLISH_DONE is the final message on the request stream once all objects are
  // delivered, so the peer learns the publisher is done rather than inferring it
  // from the stream closing.
  if result.is_ok() {
    send_publish_done(&mut request_stream, 0, stream_count(data_config)).await;
  }

  drop(request_stream);
  result
}

/// Number of subgroup streams a run opens (one per group); datagram mode opens none.
fn stream_count(config: &DataConfig) -> u64 {
  match config.delivery_mode {
    DeliveryMode::Subgroup => config.group_count,
    DeliveryMode::Datagram => 0,
  }
}

async fn send_data(
  connection: &Arc<TransportConnection>,
  track_alias: u64,
  config: &DataConfig,
  published: &Published,
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
        published,
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
        published,
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
  published: &Published,
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

      // Communicate the ID gaps explicitly.
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
          published.record(group_id, object_id).await;
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
  published: &Published,
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

      // Communicate the ID gaps explicitly: the first object
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
          published.record(group_id, object_id).await;
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
