// Copyright 2026 The MOQtail Authors
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

//! The relay's own metrics, published as MoQ tracks in the layout of "Metrics over
//! MOQT", so any MoQ client can subscribe to them through this relay or a cluster
//! peer and fetch recent history from the relay's cache.
//!
//! The publisher is an ordinary MoQ client: it connects to its own relay over raw
//! QUIC on loopback and PUBLISHes like any other, so the relay core needs nothing
//! special for it. It skips certificate verification, which is safe only because
//! the connection never leaves this machine.

use crate::ApiState;
use crate::latency::Summary;
use anyhow::{Context, Result, bail};
use bytes::Bytes;
use moqtail::metrics::{
  CATALOG_TRACK, MetricDescription, MetricKind, MetricValue, NAMESPACE_PREFIX, ValueKind,
  encode_capture_info, encode_catalog_header, encode_description, encode_value,
};
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant::{GroupOrder, SUPPORTED_VERSIONS};
use moqtail::model::control::control_message::{ControlMessage, ControlMessageTrait};
use moqtail::model::control::publish::Publish;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::request_error::RequestError;
use moqtail::model::control::setup::Setup;
use moqtail::model::data::constant::ObjectStatus;
use moqtail::model::data::object::Object;
use moqtail::model::data::subgroup_header::SubgroupHeader;
use moqtail::model::data::subgroup_object::SubgroupObject;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::parameter::setup_option::SetupOption;
use moqtail::model::property::object_property::ObjectProperty;
use moqtail::model::property::track_property::TrackProperty;
use moqtail::transport::connection::TransportConnection;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{HeaderInfo, SendDataStream};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{info, warn};
use wtransport::quinn;
use wtransport::tls;

/// The headline level: written every second, kept an hour.
const HEADLINE: Level = Level {
  track: "4",
  priority: 192,
  every: Duration::from_secs(1),
  cache: Duration::from_secs(3600),
};
/// The detail level: written every ten seconds, kept ten minutes.
const DETAIL: Level = Level {
  track: "6",
  priority: 224,
  every: Duration::from_secs(10),
  cache: Duration::from_secs(600),
};
const CATALOG_PRIORITY: u8 = 128;
/// The catalog is published again this often as a new version, so it never ages
/// out of the relay's cache while the data still references it.
const CATALOG_EVERY: Duration = Duration::from_secs(600);
const RECONNECT_AFTER: Duration = Duration::from_secs(5);

struct Level {
  track: &'static str,
  priority: u8,
  every: Duration,
  cache: Duration,
}

/// A metric of the catalog and how to read its current value.
struct Metric {
  description: MetricDescription,
  read: fn(&Sample) -> MetricValue,
}

/// What one capture reads from the API's state.
struct Sample {
  connections: u64,
  tracks: u64,
  subscriptions: u64,
  cached_groups: u64,
  counters: [u64; 7],
  total: Option<Summary>,
  stages_p99: [Option<f64>; 3],
  requests_ok: u64,
  requests_error: u64,
  messages_received: u64,
  messages_sent: u64,
  upstream: [u64; 3],
}

fn unsigned(name: &str, kind: MetricKind, unit: &str) -> MetricDescription {
  MetricDescription {
    kind,
    value: ValueKind::Unsigned,
    name: name.into(),
    unit: unit.into(),
  }
}

fn seconds(name: &str) -> MetricDescription {
  MetricDescription {
    kind: MetricKind::Gauge,
    value: ValueKind::Float,
    name: name.into(),
    unit: "s".into(),
  }
}

/// Milliseconds to seconds; 0 when nothing was measured.
fn secs(ms: Option<f64>) -> MetricValue {
  MetricValue::Float(ms.unwrap_or(0.0) / 1000.0)
}

/// The headline metrics, then the detail metrics. A metric's object id is its
/// position in the whole list, from 1.
fn catalog() -> (Vec<Metric>, Vec<Metric>) {
  use MetricKind::{Counter, Gauge};
  let headline = vec![
    Metric {
      description: unsigned("connections", Gauge, "{connection}"),
      read: |s| MetricValue::Unsigned(s.connections),
    },
    Metric {
      description: unsigned("tracks", Gauge, "{track}"),
      read: |s| MetricValue::Unsigned(s.tracks),
    },
    Metric {
      description: unsigned("subscriptions", Gauge, "{subscription}"),
      read: |s| MetricValue::Unsigned(s.subscriptions),
    },
    Metric {
      description: unsigned("objects_received", Counter, "{object}"),
      read: |s| MetricValue::Unsigned(s.counters[0]),
    },
    Metric {
      description: unsigned("bytes_received", Counter, "By"),
      read: |s| MetricValue::Unsigned(s.counters[1]),
    },
    Metric {
      description: unsigned("objects_sent", Counter, "{object}"),
      read: |s| MetricValue::Unsigned(s.counters[2]),
    },
    Metric {
      description: unsigned("bytes_sent", Counter, "By"),
      read: |s| MetricValue::Unsigned(s.counters[3]),
    },
    Metric {
      description: unsigned("objects_dropped", Counter, "{object}"),
      read: |s| MetricValue::Unsigned(s.counters[4]),
    },
    Metric {
      description: unsigned("cache_groups", Gauge, "{group}"),
      read: |s| MetricValue::Unsigned(s.cached_groups),
    },
    Metric {
      description: seconds("object_processing_total_p50"),
      read: |s| secs(s.total.as_ref().and_then(|t| t.p50)),
    },
    Metric {
      description: seconds("object_processing_total_p95"),
      read: |s| secs(s.total.as_ref().and_then(|t| t.p95)),
    },
    Metric {
      description: seconds("object_processing_total_p99"),
      read: |s| secs(s.total.as_ref().and_then(|t| t.p99)),
    },
    Metric {
      description: seconds("object_processing_total_max"),
      read: |s| secs(s.total.as_ref().and_then(|t| t.max)),
    },
    Metric {
      description: seconds("object_processing_ingest_p99"),
      read: |s| secs(s.stages_p99[0]),
    },
    Metric {
      description: seconds("object_processing_queue_p99"),
      read: |s| secs(s.stages_p99[1]),
    },
    Metric {
      description: seconds("object_processing_write_p99"),
      read: |s| secs(s.stages_p99[2]),
    },
  ];
  let detail = vec![
    Metric {
      description: unsigned("requests_ok", Counter, "{request}"),
      read: |s| MetricValue::Unsigned(s.requests_ok),
    },
    Metric {
      description: unsigned("requests_error", Counter, "{request}"),
      read: |s| MetricValue::Unsigned(s.requests_error),
    },
    Metric {
      description: unsigned("control_messages_received", Counter, "{message}"),
      read: |s| MetricValue::Unsigned(s.messages_received),
    },
    Metric {
      description: unsigned("control_messages_sent", Counter, "{message}"),
      read: |s| MetricValue::Unsigned(s.messages_sent),
    },
    Metric {
      description: unsigned("upstream_requests_ok", Counter, "{request}"),
      read: |s| MetricValue::Unsigned(s.upstream[0]),
    },
    Metric {
      description: unsigned("upstream_requests_failed", Counter, "{request}"),
      read: |s| MetricValue::Unsigned(s.upstream[1]),
    },
    Metric {
      description: unsigned("upstream_requests_timed_out", Counter, "{request}"),
      read: |s| MetricValue::Unsigned(s.upstream[2]),
    },
    Metric {
      description: unsigned("streams_opened", Counter, "{stream}"),
      read: |s| MetricValue::Unsigned(s.counters[5]),
    },
    Metric {
      description: unsigned("subscriptions_too_far_behind", Counter, "{subscription}"),
      read: |s| MetricValue::Unsigned(s.counters[6]),
    },
  ];
  (headline, detail)
}

async fn take_sample(state: &ApiState) -> Sample {
  let connections = state.open_connections().await;
  let tracks = state.tracks().await;
  let counters = state.counters.as_deref().map_or([0; 7], |c| {
    [
      c.objects_received.get(),
      c.bytes_received.get(),
      c.objects_sent.get() + c.fetch_objects_sent.get(),
      c.bytes_sent.get() + c.fetch_bytes_sent.get(),
      c.dropped_queue_full.get() + c.dropped_delivery_timeout.get(),
      c.streams_opened.get(),
      c.subscriptions_too_far_behind.get(),
    ]
  });
  let window = HEADLINE.every.max(Duration::from_secs(10));
  let latency = state.probe.latency.as_ref();
  let stage_p99 = |stage| latency.and_then(|l| l.stage_summary(stage, window).p99);
  let (mut ok, mut error) = (0, 0);
  for (_, _, results) in state.control.results_by_kind() {
    ok += results.ok;
    error += results.error.values().sum::<u64>();
  }
  let totals = state.control.totals();
  let messages = |direction: &str| {
    totals
      .messages
      .iter()
      .filter(|((d, _), _)| *d == direction)
      .map(|(_, n)| n)
      .sum()
  };
  Sample {
    connections: connections.len() as u64,
    tracks: tracks.len() as u64,
    subscriptions: connections
      .iter()
      .map(|c| c.subscriptions.len() as u64)
      .sum(),
    cached_groups: tracks.iter().map(|t| t.cached_groups).sum(),
    counters,
    total: latency.map(|l| l.stage_summary(3, window)),
    stages_p99: [stage_p99(0), stage_p99(1), stage_p99(2)],
    requests_ok: ok,
    requests_error: error,
    messages_received: messages("received"),
    messages_sent: messages("sent"),
    upstream: [
      totals.upstream_ok,
      totals.upstream_failed,
      totals.upstream_timed_out,
    ],
  }
}

/// Publishes the metrics tracks for as long as the API runs, connecting again
/// after the relay closes the connection.
pub(crate) async fn run(state: Arc<ApiState>, relay: SocketAddr, resource_id: String) {
  info!("Publishing relay metrics as metrics.moq.arpa/v1/{resource_id}");
  loop {
    if let Err(e) = publish(&state, relay, &resource_id).await {
      warn!("Relay metrics publisher stopped: {e:#}; connecting again");
    }
    tokio::time::sleep(RECONNECT_AFTER).await;
  }
}

async fn publish(state: &Arc<ApiState>, relay: SocketAddr, resource_id: &str) -> Result<()> {
  let connection = Arc::new(connect(relay).await?);
  // The relay may ask this publisher for history it no longer caches. History is
  // served from the cache only, so every request is refused.
  let refusing = tokio::spawn(refuse_requests(connection.clone()));

  let namespace = Tuple::from_utf8_path(&format!(
    "{}/{}/{resource_id}",
    NAMESPACE_PREFIX[0], NAMESPACE_PREFIX[1]
  ));
  let mut requests = Vec::new();
  requests.push(
    request(
      &connection,
      ControlMessage::PublishNamespace(Box::new(PublishNamespace::new(0, namespace.clone(), &[]))),
    )
    .await?,
  );
  let catalog_alias = 1;
  requests.push(
    publish_track(
      &connection,
      2,
      &namespace,
      CATALOG_TRACK,
      catalog_alias,
      CATALOG_PRIORITY,
      DETAIL.cache,
    )
    .await?,
  );
  requests.push(
    publish_track(
      &connection,
      4,
      &namespace,
      HEADLINE.track,
      2,
      HEADLINE.priority,
      HEADLINE.cache,
    )
    .await?,
  );
  requests.push(
    publish_track(
      &connection,
      6,
      &namespace,
      DETAIL.track,
      3,
      DETAIL.priority,
      DETAIL.cache,
    )
    .await?,
  );

  let (headline, detail) = catalog();
  let mut catalog_version = now_ms();
  write_catalog(
    &connection,
    catalog_alias,
    catalog_version,
    &headline,
    &detail,
  )
  .await?;

  let mut headline_tick = tokio::time::interval(HEADLINE.every);
  let mut detail_tick = tokio::time::interval(DETAIL.every);
  let mut catalog_tick = tokio::time::interval(CATALOG_EVERY);
  catalog_tick.tick().await;
  let (mut last_headline, mut last_detail) = (None, None);
  let result: Result<()> = loop {
    tokio::select! {
      _ = headline_tick.tick() => {
        let sample = take_sample(state).await;
        if let Err(e) = write_sample(&connection, 2, HEADLINE.priority, catalog_version, 1, &headline, &sample, &mut last_headline).await {
          break Err(e);
        }
      }
      _ = detail_tick.tick() => {
        let sample = take_sample(state).await;
        let first_id = headline.len() as u64 + 1;
        if let Err(e) = write_sample(&connection, 3, DETAIL.priority, catalog_version, first_id, &detail, &sample, &mut last_detail).await {
          break Err(e);
        }
      }
      _ = catalog_tick.tick() => {
        catalog_version = now_ms();
        if let Err(e) = write_catalog(&connection, catalog_alias, catalog_version, &headline, &detail).await {
          break Err(e);
        }
      }
      _ = connection.closed() => break Ok(()),
    }
  };
  refusing.abort();
  drop(requests);
  result
}

async fn connect(relay: SocketAddr) -> Result<TransportConnection> {
  let mut tls_config = tls::client::build_default_tls_config(
    Arc::new(tls::rustls::RootCertStore::empty()),
    Some(Arc::new(tls::client::NoServerVerification::new())),
  );
  tls_config.alpn_protocols = SUPPORTED_VERSIONS
    .split(',')
    .map(|v| v.trim().as_bytes().to_vec())
    .collect();
  let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?;
  let mut transport = quinn::TransportConfig::default();
  transport.keep_alive_interval(Some(Duration::from_secs(3)));
  let mut client = quinn::ClientConfig::new(Arc::new(crypto));
  client.transport_config(Arc::new(transport));
  let local: SocketAddr = if relay.is_ipv6() {
    "[::]:0"
  } else {
    "0.0.0.0:0"
  }
  .parse()?;
  let endpoint = quinn::Endpoint::client(local)?;
  let quic = endpoint
    .connect_with(client, relay, "localhost")?
    .await
    .with_context(|| format!("cannot reach the relay at {relay}"))?;
  let connection = TransportConnection::Quic(quic);

  // SETUP on a pair of unidirectional streams, ours sent before theirs is accepted.
  let mut send = connection.open_uni().await?;
  let setup = Setup::new(vec![
    SetupOption::new_authority(format!("localhost:{}", relay.port())).try_into()?,
    SetupOption::new_path(String::new()).try_into()?,
    SetupOption::new_moqt_implementation("moqtail-relay-metrics".into()).try_into()?,
  ]);
  send
    .write_all(&setup.serialize().map_err(|e| anyhow::anyhow!("{e:?}"))?)
    .await
    .map_err(|e| anyhow::anyhow!("cannot send SETUP: {e:?}"))?;
  let recv = connection.accept_uni().await?;
  let mut control = ControlStreamHandler::new(send, recv);
  control
    .read_setup()
    .await
    .map_err(|e| anyhow::anyhow!("relay refused SETUP: {e:?}"))?;
  // The control stream stays open for the session; its handler is dropped with it.
  tokio::spawn(async move { while control.next_message().await.is_ok() {} });
  Ok(connection)
}

/// Sends a request on its own bidi stream and waits for REQUEST_OK. Returns the
/// stream, which must stay open for as long as the request holds.
async fn request(
  connection: &TransportConnection,
  message: ControlMessage,
) -> Result<ControlStreamHandler> {
  let (send, recv) = connection.open_bi().await?;
  let mut stream = ControlStreamHandler::new(send, recv);
  stream
    .send(&message)
    .await
    .map_err(|e| anyhow::anyhow!("cannot send {:?}: {e:?}", message.get_type()))?;
  match stream.next_message().await {
    Ok(ControlMessage::RequestOk(_)) => Ok(stream),
    Ok(other) => bail!("relay answered {:?} with {other:?}", message.get_type()),
    Err(e) => bail!("relay closed {:?}: {e:?}", message.get_type()),
  }
}

async fn publish_track(
  connection: &TransportConnection,
  request_id: u64,
  namespace: &Tuple,
  name: &str,
  alias: u64,
  priority: u8,
  cache: Duration,
) -> Result<ControlStreamHandler> {
  let properties = vec![
    TrackProperty::DefaultPublisherPriority { priority },
    TrackProperty::DefaultPublisherGroupOrder {
      order: GroupOrder::Ascending,
    },
    TrackProperty::MaxCacheDuration {
      duration_ms: cache.as_millis() as u64,
    },
  ];
  let publish = Publish::new(
    request_id,
    namespace.clone(),
    TupleField::from_utf8(name),
    alias,
    vec![],
    properties,
  );
  request(connection, ControlMessage::Publish(Box::new(publish))).await
}

async fn refuse_requests(connection: Arc<TransportConnection>) {
  while let Ok((send, recv)) = connection.accept_bi().await {
    tokio::spawn(async move {
      let mut stream = ControlStreamHandler::new(send, recv);
      if stream.next_message().await.is_ok() {
        let refusal = RequestError::new(
          RequestErrorCode::DoesNotExist,
          0,
          ReasonPhrase::try_new("metrics history is served from the relay's cache".into())
            .expect("short reason"),
        );
        let _ = stream
          .send(&ControlMessage::RequestError(Box::new(refusal)))
          .await;
        stream.finish().await;
      }
    });
  }
}

/// Writes one group on its own stream: the given payloads as objects `first_id`,
/// `first_id + 1`, ..., object 0 written before them when `capture` is set, then an
/// end-of-group object. `group_gap` and the gap before `first_id` are signalled
/// with the Prior Group and Object ID Gap properties.
#[allow(clippy::too_many_arguments)]
async fn write_group(
  connection: &TransportConnection,
  alias: u64,
  priority: u8,
  group: u64,
  group_gap: Option<u64>,
  capture: Option<Bytes>,
  first_id: u64,
  payloads: Vec<Bytes>,
) -> Result<()> {
  let stream = Arc::new(Mutex::new(connection.open_uni().await?));
  let header =
    SubgroupHeader::new_with_explicit_id(alias, group, 0, Some(priority), true, true, true);
  let mut writer = SendDataStream::new(stream.clone(), HeaderInfo::Subgroup { header })
    .await
    .map_err(|e| anyhow::anyhow!("cannot open a metrics stream: {e:?}"))?;
  let mut previous = None;
  let mut objects: Vec<(u64, Option<Bytes>, Vec<ObjectProperty>)> = Vec::new();
  let mut group_gap = group_gap.filter(|g| *g > 0);
  let mut take_group_gap = || {
    group_gap
      .take()
      .map(|gap| vec![ObjectProperty::PriorGroupIdGap { gap }])
      .unwrap_or_default()
  };
  if let Some(capture) = capture {
    objects.push((0, Some(capture), take_group_gap()));
  }
  for (i, payload) in payloads.into_iter().enumerate() {
    let id = first_id + i as u64;
    let mut properties = take_group_gap();
    let expected = previous_id(&objects).map_or(0, |p| p + 1);
    if id > expected {
      properties.push(ObjectProperty::PriorObjectIdGap { gap: id - expected });
    }
    objects.push((id, Some(payload), properties));
  }
  let end = previous_id(&objects).map_or(0, |p| p + 1);
  objects.push((end, None, vec![]));

  for (id, payload, properties) in objects {
    let status = if payload.is_none() {
      Some(ObjectStatus::EndOfGroup)
    } else {
      None
    };
    let object = Object::try_from_subgroup(
      SubgroupObject {
        object_id: id,
        properties: Some(properties),
        object_status: status,
        payload,
      },
      alias,
      group,
      Some(0),
      Some(priority),
      priority,
    )
    .map_err(|e| anyhow::anyhow!("cannot build a metrics object: {e:?}"))?;
    writer
      .send_object(&object, previous)
      .await
      .map_err(|e| anyhow::anyhow!("cannot write a metrics object: {e:?}"))?;
    previous = Some(id);
  }
  writer
    .finish()
    .await
    .map_err(|e| anyhow::anyhow!("cannot finish a metrics stream: {e:?}"))?;
  Ok(())
}

fn previous_id(objects: &[(u64, Option<Bytes>, Vec<ObjectProperty>)]) -> Option<u64> {
  objects.last().map(|(id, ..)| *id)
}

/// The catalog as one group whose id is its version.
async fn write_catalog(
  connection: &TransportConnection,
  alias: u64,
  version: u64,
  headline: &[Metric],
  detail: &[Metric],
) -> Result<()> {
  let header = encode_catalog_header(&[
    ("service.name", "moqtail-relay"),
    ("service.version", env!("CARGO_PKG_VERSION")),
  ])
  .map_err(|e| anyhow::anyhow!("{e:?}"))?;
  let mut payloads = Vec::new();
  for metric in headline.iter().chain(detail) {
    payloads.push(encode_description(&metric.description).map_err(|e| anyhow::anyhow!("{e:?}"))?);
  }
  write_group(
    connection,
    alias,
    CATALOG_PRIORITY,
    version,
    None,
    Some(header),
    1,
    payloads,
  )
  .await
}

/// One capture as one group whose id is the capture time in milliseconds.
#[allow(clippy::too_many_arguments)]
async fn write_sample(
  connection: &TransportConnection,
  alias: u64,
  priority: u8,
  catalog_version: u64,
  first_id: u64,
  metrics: &[Metric],
  sample: &Sample,
  last_group: &mut Option<u64>,
) -> Result<()> {
  let (group, offset_nanos) = capture_time();
  if last_group.is_some_and(|last| group <= last) {
    return Ok(());
  }
  let capture =
    encode_capture_info(offset_nanos, catalog_version).map_err(|e| anyhow::anyhow!("{e:?}"))?;
  let mut payloads = Vec::new();
  for metric in metrics {
    payloads.push(encode_value((metric.read)(sample)).map_err(|e| anyhow::anyhow!("{e:?}"))?);
  }
  let group_gap = last_group.map(|last| group - last - 1);
  write_group(
    connection,
    alias,
    priority,
    group,
    group_gap,
    Some(capture),
    first_id,
    payloads,
  )
  .await?;
  *last_group = Some(group);
  Ok(())
}

/// The capture time as whole milliseconds and the nanoseconds past them.
fn capture_time() -> (u64, u64) {
  let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default();
  (
    now.as_millis() as u64,
    u64::from(now.subsec_nanos() % 1_000_000),
  )
}

fn now_ms() -> u64 {
  capture_time().0
}

/// The resource id: the configured one, or one made up for this process.
pub fn resource_id(configured: Option<&str>) -> Result<String> {
  match configured {
    Some(id) => {
      if id.len() != 16
        || !id
          .bytes()
          .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
      {
        bail!("metrics_resource_id must be 16 lowercase hexadecimal characters");
      }
      Ok(id.to_string())
    }
    None => {
      use std::hash::{BuildHasher, Hasher};
      let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
      hasher.write_u64(now_ms());
      hasher.write_u32(std::process::id());
      Ok(format!("{:016x}", hasher.finish()))
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resource_ids_are_sixteen_hex_characters() {
    assert_eq!(
      resource_id(Some("0123456789abcdef")).unwrap(),
      "0123456789abcdef"
    );
    assert!(resource_id(Some("0123456789ABCDEF")).is_err());
    assert!(resource_id(Some("short")).is_err());
    let made_up = resource_id(None).unwrap();
    assert_eq!(made_up.len(), 16);
  }

  #[test]
  fn catalog_names_are_unique() {
    let (headline, detail) = catalog();
    let mut names: Vec<&str> = headline
      .iter()
      .chain(&detail)
      .map(|m| m.description.name.as_str())
      .collect();
    let total = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), total);
  }
}
