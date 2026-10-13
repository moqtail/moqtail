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

//! The two prefix subscriptions: SUBSCRIBE_NAMESPACE, which discovers the
//! namespaces under a prefix, and SUBSCRIBE_TRACKS, which asks for a PUBLISH of
//! every track under one. They share a shape -- one request on its own bidi
//! stream, a long-lived response stream, and cancellation by resetting that
//! stream -- and differ in what arrives on the stream afterwards.

use crate::cli::PrefixKind;
use crate::connection::MoqConnection;
use crate::filter_spec::check_limit;
use crate::stats::ReceptionStats;
use crate::utils::should_log;
use anyhow::Result;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::request_ok::RequestOk;
use moqtail::model::control::subscribe_namespace::SubscribeNamespace;
use moqtail::model::control::subscribe_tracks::SubscribeTracks;
use moqtail::model::error::{StreamResetCode, TerminationCode};
use moqtail::model::parameter::combined_filter::{CombinedFilter, ObjectFields};
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::transport::connection::TransportConnection;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{HeaderInfo, RecvDataStream};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub struct PrefixSubscribeConfig {
  pub namespace: String,
  pub duration: u64,
  pub kind: PrefixKind,
  pub overlap: Option<PrefixKind>,
  /// False asks for each PUBLISH without Track Properties (SUBSCRIBE_TRACKS only).
  pub include_properties: bool,
  /// FORWARD, LOCATION_FILTER and FILL_PARAMETERS for each pushed track's
  /// subscription (SUBSCRIBE_TRACKS only).
  pub pushed_track_parameters: Vec<MessageParameter>,
  /// Range Filters for SUBSCRIBE_TRACKS, track filters included.
  pub range_filters: Vec<MessageParameter>,
  pub ignore_filter_limit: bool,
}

/// What a prefix subscription is called in the log, so two of them in one run
/// read apart.
fn label(kind: PrefixKind) -> &'static str {
  match kind {
    PrefixKind::Namespace => "SUBSCRIBE_NAMESPACE",
    PrefixKind::Tracks => "SUBSCRIBE_TRACKS",
  }
}

/// What a SUBSCRIBE_TRACKS run knows about one pushed track.
struct TrackState {
  name: String,
  /// Its own sequence: `ReceptionStats` follows one track, and a prefix
  /// subscription receives many at once, whose objects interleave. Counting them
  /// together would report every switch between two tracks as a gap.
  stats: ReceptionStats,
  /// What the Range Filters let through for this track, once its PUBLISH has said
  /// what its Track Properties are. Empty when there are no filters.
  filter: CombinedFilter,
}

impl TrackState {
  fn new(name: String) -> Self {
    Self {
      name,
      stats: ReceptionStats::new(),
      filter: CombinedFilter::default(),
    }
  }
}

/// The pushed tracks by the track alias their objects carry, so an object
/// arriving on a uni stream finds the track it belongs to.
type Tracks = Arc<Mutex<HashMap<u64, TrackState>>>;

/// The Range Filters SUBSCRIBE_TRACKS asked for, and the pushed tracks they
/// should have kept back.
#[derive(Clone, Default)]
struct TrackFilter {
  filter: CombinedFilter,
  excluded_pushes: Arc<Mutex<Vec<String>>>,
}

pub async fn run(moq: MoqConnection, config: PrefixSubscribeConfig) -> Result<()> {
  // Keep `moq` alive for the whole run: the session's control stream carries only
  // SETUP, but closing it would end the session under the subscription.
  let connection = moq.connection.clone();
  let label = label(config.kind);
  let prefix = Tuple::from_utf8_path(&config.namespace);
  info!("{label} for namespace prefix: {}", config.namespace);

  let parameters = if config.kind == PrefixKind::Tracks {
    check_limit(
      &config.range_filters,
      moq.peer_max_filter_ranges,
      config.ignore_filter_limit,
    )?;
    config.range_filters.clone()
  } else {
    vec![]
  };
  let track_filter = TrackFilter {
    filter: CombinedFilter::new(&parameters).unwrap_or_else(|e| {
      warn!("Requesting range filters the server has to refuse ({e})");
      CombinedFilter::default()
    }),
    ..Default::default()
  };
  let mut parameters = parameters;
  if config.kind == PrefixKind::Tracks {
    if !config.include_properties {
      parameters.push(MessageParameter::new_include_properties(false));
    }
    parameters.extend(config.pushed_track_parameters.iter().cloned());
  }
  let mut request_stream =
    send_request(&connection, config.kind, prefix.clone(), 0, parameters).await?;

  // Tracks are pushed as soon as the request is registered, so the receivers are
  // in place before the first response is read. A discovery subscription is
  // answered entirely on its own stream and needs neither.
  let tracks = if config.kind == PrefixKind::Tracks {
    let tracks: Tracks = Arc::new(Mutex::new(HashMap::new()));
    accept_pushed_publishes(connection.clone(), tracks.clone(), track_filter.clone());
    receive_objects(connection.clone(), tracks.clone());
    Some(tracks)
  } else {
    None
  };

  // The first response settles whether this subscription exists, and a second
  // request is only meaningful against one that does -- so it waits for that
  // answer rather than racing the relay's registration of the first.
  let alive = read_one(&mut request_stream, label).await;

  let mut overlap_stream = match config.overlap {
    Some(kind) if alive => {
      let overlap_label = label_overlap(kind);
      info!("{overlap_label} on the same prefix and session, to test overlap");
      let mut stream = send_request(&connection, kind, prefix, 2, vec![]).await?;
      read_one(&mut stream, overlap_label).await;
      Some(stream)
    }
    _ => None,
  };

  if alive {
    // A prefix subscription lives for exactly as long as its request stream, so the
    // run ends by resetting that stream: the relay drops the subscription and frees
    // the prefix while the session stays open. Letting the process exit instead
    // tears down the whole connection, which says nothing about this one request.
    let deadline =
      (config.duration > 0).then(|| Instant::now() + Duration::from_secs(config.duration));
    let overlap = overlap_stream
      .as_mut()
      .map(|stream| (stream, label_overlap(config.overlap.unwrap())));

    if read_responses(&mut request_stream, label, overlap, deadline).await {
      info!("{label}: duration elapsed, cancelling by resetting the request stream");
      request_stream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
      if let Some(stream) = overlap_stream {
        stream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
      }
    }
  }

  if let Some(tracks) = tracks {
    let tracks = tracks.lock().unwrap();
    if tracks.is_empty() {
      info!("{label}: no track was pushed under this prefix");
    }
    for state in tracks.values() {
      info!(
        "{label}: {} objects received={}, gaps={}, filter_violations={}",
        state.name,
        state.stats.total_received,
        state.stats.sequence_gaps,
        state.stats.filter_violations
      );
    }
    let excluded = track_filter.excluded_pushes.lock().unwrap();
    if !excluded.is_empty() {
      warn!(
        "{label}: {} tracks pushed that the track filter excludes: {:?}",
        excluded.len(),
        *excluded
      );
    }
  }
  info!("{label} complete");
  Ok(())
}

/// How the second, overlapping request is labelled, so its responses are never
/// mistaken for the first one's.
fn label_overlap(kind: PrefixKind) -> &'static str {
  match kind {
    PrefixKind::Namespace => "overlapping SUBSCRIBE_NAMESPACE",
    PrefixKind::Tracks => "overlapping SUBSCRIBE_TRACKS",
  }
}

/// Open a bidi stream for one prefix subscription and send its request on it.
async fn send_request(
  connection: &Arc<TransportConnection>,
  kind: PrefixKind,
  prefix: Tuple,
  request_id: u64,
  parameters: Vec<MessageParameter>,
) -> Result<ControlStreamHandler> {
  let request = match kind {
    PrefixKind::Namespace => ControlMessage::SubscribeNamespace(Box::new(SubscribeNamespace::new(
      request_id, prefix, parameters,
    ))),
    PrefixKind::Tracks => ControlMessage::SubscribeTracks(Box::new(SubscribeTracks::new(
      request_id, prefix, parameters,
    ))),
  };

  let (send, recv) = connection.open_bi().await?;
  let mut request_stream = ControlStreamHandler::new(send, recv);
  request_stream
    .send(&request)
    .await
    .map_err(|e| anyhow::anyhow!("Failed to send {}: {e:?}", label(kind)))?;
  Ok(request_stream)
}

/// Read and log one message. Returns false once the stream has no more to give,
/// which for the first message means the request never took.
async fn read_one(request_stream: &mut ControlStreamHandler, label: &str) -> bool {
  match request_stream.next_message().await {
    Ok(msg) => {
      log_response(label, msg);
      true
    }
    Err(e) => {
      info!("{label}: response stream ended: {e:?}");
      false
    }
  }
}

/// Read both subscriptions' response streams until the primary ends, or until
/// `deadline` passes. Returns true when the deadline is what stopped it, i.e. the
/// caller still holds live subscriptions and has to cancel them.
async fn read_responses(
  request_stream: &mut ControlStreamHandler,
  label: &str,
  overlap: Option<(&mut ControlStreamHandler, &str)>,
  deadline: Option<Instant>,
) -> bool {
  let timer = async {
    match deadline {
      Some(deadline) => tokio::time::sleep_until(deadline).await,
      None => std::future::pending::<()>().await,
    }
  };
  tokio::pin!(timer);

  let (mut overlap_stream, overlap_label) = match overlap {
    Some((stream, label)) => (Some(stream), label),
    None => (None, ""),
  };
  let mut overlap_alive = overlap_stream.is_some();

  loop {
    tokio::select! {
      _ = &mut timer => return true,
      result = request_stream.next_message() => match result {
        Ok(msg) => log_response(label, msg),
        Err(e) => {
          info!("{label}: response stream ended: {e:?}");
          return false;
        }
      },
      result = next_message(&mut overlap_stream), if overlap_alive => match result {
        Ok(msg) => log_response(overlap_label, msg),
        Err(e) => {
          info!("{overlap_label}: response stream ended: {e:?}");
          overlap_alive = false;
        }
      },
    }
  }
}

/// The next message on a stream that may not exist. Never resolving in that case
/// keeps it out of a `select!` without a second copy of the loop.
async fn next_message(
  stream: &mut Option<&mut ControlStreamHandler>,
) -> Result<ControlMessage, TerminationCode> {
  match stream {
    Some(stream) => stream.next_message().await,
    None => std::future::pending().await,
  }
}

/// Log one message from a response stream. REQUEST_ERROR is spelled out: its code
/// is the answer to whether two prefix subscriptions were allowed to overlap, and
/// digging it out of a raw message dump is needless work.
fn log_response(label: &str, msg: ControlMessage) {
  match msg {
    ControlMessage::RequestOk(_) => info!("{label}: REQUEST_OK, subscription is live"),
    ControlMessage::RequestError(m) => warn!(
      "{label}: REQUEST_ERROR code={:?} retry_interval={} reason={:?}",
      m.error_code,
      m.retry_interval,
      m.reason_phrase.as_str()
    ),
    ControlMessage::Namespace(m) => info!(
      "{label}: NAMESPACE suffix={}",
      m.track_namespace_suffix.to_utf8_path()
    ),
    ControlMessage::NamespaceDone(m) => info!(
      "{label}: NAMESPACE_DONE suffix={}",
      m.track_namespace_suffix.to_utf8_path()
    ),
    ControlMessage::PublishSkipped(m) => info!(
      "{label}: PUBLISH_SKIPPED namespace_suffix={} track_name={}",
      m.track_namespace_suffix.to_utf8_path(),
      m.track_name.as_str()
    ),
    other => info!("{label}: received {:?}", other.get_type()),
  }
}

/// Accept the PUBLISH the relay pushes for each matching track, each on its own
/// bidi stream, and answer REQUEST_OK so the track starts flowing. The alias the
/// PUBLISH carries is recorded, so `receive_objects` can name the track an object
/// belongs to; the stream is then held open, since PUBLISH_DONE ends the request
/// there.
fn accept_pushed_publishes(
  connection: Arc<TransportConnection>,
  tracks: Tracks,
  track_filter: TrackFilter,
) {
  tokio::spawn(async move {
    loop {
      let (send, recv) = match connection.accept_bi().await {
        Ok(streams) => streams,
        Err(e) => {
          info!("SUBSCRIBE_TRACKS: stopped accepting pushed tracks: {e:?}");
          return;
        }
      };

      let tracks = tracks.clone();
      let track_filter = track_filter.clone();
      tokio::spawn(async move {
        let mut handler = ControlStreamHandler::new(send, recv);
        let name = match handler.next_message().await {
          Ok(ControlMessage::Publish(m)) => {
            let name = format!(
              "{}/{}",
              m.track_namespace.to_utf8_path(),
              m.track_name.as_str()
            );
            info!(
              "SUBSCRIBE_TRACKS: PUBLISH for {name} (track_alias={}, parameters={:?}, track_properties={:?})",
              m.track_alias, m.parameters, m.track_properties
            );
            // The track filter decides whether this track should have been pushed,
            // and which of its objects then should be.
            let filter = match track_filter.filter.narrow_for_track(&m.track_properties) {
              Some(filter) => filter,
              None => {
                warn!(
                  "SUBSCRIBE_TRACKS: {name} was pushed, but its Track Properties {:?} pass no track filter",
                  m.track_properties
                );
                track_filter
                  .excluded_pushes
                  .lock()
                  .unwrap()
                  .push(name.clone());
                CombinedFilter::default()
              }
            };
            // An object can beat its PUBLISH here, so the entry may already exist
            // with a placeholder name and a sequence worth keeping.
            {
              let mut tracks = tracks.lock().unwrap();
              let state = tracks
                .entry(m.track_alias)
                .or_insert_with(|| TrackState::new(name.clone()));
              state.name = name.clone();
              state.filter = filter;
            }
            // An empty parameter list accepts the subscription as the relay
            // proposed it, which forwards by default.
            if let Err(e) = handler
              .send(&ControlMessage::RequestOk(Box::new(RequestOk::new(vec![]))))
              .await
            {
              warn!("SUBSCRIBE_TRACKS: failed to accept PUBLISH for {name}: {e:?}");
              return;
            }
            name
          }
          Ok(other) => {
            warn!(
              "SUBSCRIBE_TRACKS: unexpected {:?} on a pushed stream",
              other.get_type()
            );
            return;
          }
          Err(e) => {
            info!("SUBSCRIBE_TRACKS: pushed stream ended: {e:?}");
            return;
          }
        };

        loop {
          match handler.next_message().await {
            Ok(ControlMessage::PublishDone(m)) => info!(
              "SUBSCRIBE_TRACKS: PUBLISH_DONE for {name}: {:?}",
              m.status_code
            ),
            Ok(other) => info!("SUBSCRIBE_TRACKS: {:?} for {name}", other.get_type()),
            Err(e) => {
              debug!("SUBSCRIBE_TRACKS: stream for {name} closed: {e:?}");
              return;
            }
          }
        }
      });
    }
  });
}

/// Receive the objects of every pushed track on the session's uni streams,
/// counting them into one set of stats and naming each track by the alias its
/// PUBLISH announced.
fn receive_objects(connection: Arc<TransportConnection>, tracks: Tracks) {
  tokio::spawn(async move {
    // SUBSCRIBE_TRACKS issues no FETCH, so nothing is ever waiting in here; the
    // stream reader takes it to tell a fetch's objects from a subscription's. A fill
    // fetch stream needs no entry: it is named by the pushed PUBLISH's Request ID.
    let pending_fetches = Arc::new(RwLock::new(BTreeMap::new()));

    loop {
      let stream = match connection.accept_uni().await {
        Ok(stream) => stream,
        Err(e) => {
          info!("SUBSCRIBE_TRACKS: object stream accept ended: {e:?}");
          return;
        }
      };

      let tracks = tracks.clone();
      let pending_fetches = pending_fetches.clone();
      tokio::spawn(async move {
        let stream_handler = RecvDataStream::new(stream, pending_fetches);
        let mut handler = &stream_handler;
        let mut fill_objects = 0u64;
        loop {
          let (next_handler, object) = handler.next_object().await;
          let Some(obj) = object else {
            if fill_objects > 0 {
              info!("SUBSCRIBE_TRACKS: fill stream closed after {fill_objects} objects");
            }
            debug!("SUBSCRIBE_TRACKS: object stream closed");
            return;
          };

          // A fill's objects carry no track alias, so they are logged on their own
          // rather than counted into a track's stats.
          if let Some(HeaderInfo::Fetch { header, .. }) = stream_handler.get_header_info().await {
            fill_objects += 1;
            info!(
              "SUBSCRIBE_TRACKS: fill object {fill_objects} for PUBLISH request {}: group={}, object={}",
              header.request_id, obj.location.group, obj.location.object
            );
            handler = next_handler;
            continue;
          }

          let (name, total, status) = {
            let mut tracks = tracks.lock().unwrap();
            let state = tracks
              .entry(obj.track_alias)
              .or_insert_with(|| TrackState::new(format!("alias={}", obj.track_alias)));
            let status = if state.filter.is_empty() {
              let ok = state.stats.record_object(
                obj.track_alias,
                obj.location.group,
                obj.location.object,
                ReceptionStats::prior_group_gap(obj.properties.as_ref()),
              );
              if ok { "OK" } else { "GAP" }
            } else {
              let passed = state.filter.matches(ObjectFields::from(&obj));
              state.stats.record_filtered_object(
                obj.track_alias,
                obj.location.group,
                obj.location.object,
                passed,
              );
              if passed { "OK" } else { "FILTER_VIOLATION" }
            };
            (state.name.clone(), state.stats.total_received, status)
          };
          let sequence_ok = status == "OK";

          if should_log(total) || !sequence_ok {
            info!(
              "SUBSCRIBE_TRACKS: object {total}: track={name} group={}, object={}, seq={status}",
              obj.location.group, obj.location.object,
            );
          } else {
            debug!(
              "SUBSCRIBE_TRACKS: object {total}: track={name} group={}, object={}",
              obj.location.group, obj.location.object
            );
          }
          handler = next_handler;
        }
      });
    }
  });
}
