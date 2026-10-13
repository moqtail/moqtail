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

use crate::cli::{CliFilter, DeliveryMode};
use crate::connection::MoqConnection;
use crate::filter_spec::{check_limit, object_filter};
use crate::routing::{Router, Window};
use crate::stats::ReceptionStats;
use crate::utils::should_log;
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant::{GroupOrder, SwitchMode};
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::request_update::RequestUpdate;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::constant::DEFAULT_PUBLISHER_PRIORITY;
use moqtail::model::data::datagram::Datagram;
use moqtail::model::parameter::combined_filter::ObjectFields;
use moqtail::model::parameter::location_filter::LocationFilter;
use moqtail::model::parameter::message_parameter::{
  MessageParameter, apply_message_parameter_update,
};
use moqtail::transport::connection::TransportConnection;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::RecvDataStream;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// The Location Filter a `--filter` choice asks for.
fn location_filter(config: &SubscribeConfig) -> LocationFilter {
  let start = Location::new(config.filter_start_group, config.filter_start_object);
  let end_group = config.filter_start_group + config.end_group_delta;
  match config.filter {
    CliFilter::None => LocationFilter::None,
    CliFilter::RelativeStart => LocationFilter::RelativeStart {
      start_group: config.filter_start_group,
    },
    CliFilter::AbsoluteStart => LocationFilter::AbsoluteStart { start },
    CliFilter::AbsoluteStartGroupEnd => LocationFilter::AbsoluteStartGroupEnd { start, end_group },
    CliFilter::AbsoluteRange => LocationFilter::AbsoluteRange {
      start,
      end: Location::new(end_group, config.filter_end_object),
    },
    CliFilter::NextObject => LocationFilter::NextObject,
  }
}

/// The filter parameters a subscribe carries: LOCATION_FILTER, any Range
/// Filters, and FILL_PARAMETERS when `--fill` asks for what is already published
/// as well.
fn filter_parameters(config: &SubscribeConfig) -> Vec<MessageParameter> {
  let mut params = vec![MessageParameter::new_location_filter(location_filter(
    config,
  ))];
  params.extend(config.range_filters.iter().cloned());
  if config.fill {
    // Without a filter of its own the fill covers the subscription's.
    let mut fill: Vec<_> = config
      .fill_start_group
      .map(|start_group| {
        MessageParameter::new_location_filter(LocationFilter::RelativeStart { start_group })
      })
      .into_iter()
      .collect();
    fill.extend(config.fill_range_filters.iter().cloned());
    params.push(MessageParameter::new_fill_parameters(fill));
  }
  params
}

/// Subscribe to one track on its own bidirectional request stream. Returns the
/// assigned track alias, the Largest Object the publisher reported, and the
/// request-stream handler, which the caller keeps alive for the subscription's
/// lifetime.
#[allow(clippy::too_many_arguments)]
async fn subscribe_track(
  connection: &Arc<TransportConnection>,
  namespace: &str,
  track_name: &str,
  request_id: u64,
  subscriber_priority: u8,
  group_order: GroupOrder,
  forward: bool,
  filters: Vec<MessageParameter>,
  switch_from: Option<MessageParameter>,
) -> Result<(u64, Option<Location>, ControlStreamHandler)> {
  let ns = Tuple::from_utf8_path(namespace);
  info!(
    "Subscribing to track: {}/{} (request_id={}, priority={}, forward={}, filters={:?})",
    namespace, track_name, request_id, subscriber_priority, forward, filters
  );
  // A switch decides the Forward State itself, so FORWARD must not go with it.
  let mut parameters = vec![
    MessageParameter::new_subscriber_priority(subscriber_priority),
    MessageParameter::new_group_order(group_order),
  ];
  parameters.extend(filters);
  match switch_from {
    Some(param) => parameters.push(param),
    None => parameters.push(MessageParameter::new_forward(forward)),
  }
  let subscribe = Subscribe::new(
    request_id,
    ns,
    TupleField::from_utf8(track_name),
    parameters,
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
        "Subscribed: track={} track_alias={} track_properties={:?}",
        track_name, m.track_alias, m.track_properties
      );
      let largest = m.subscribe_parameters.iter().find_map(|p| match p {
        MessageParameter::LargestObject { location } => Some(location.clone()),
        _ => None,
      });
      Ok((m.track_alias, largest, request_stream))
    }
    Ok(ControlMessage::RequestError(m)) => {
      anyhow::bail!("SUBSCRIBE for {} refused: {:?}", track_name, m)
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
  /// False asks for the SUBSCRIBE_OK without Track Properties.
  pub include_properties: bool,
  pub update_forward_after: u64,
  pub filter: CliFilter,
  pub filter_start_group: u64,
  pub filter_start_object: u64,
  pub end_group_delta: u64,
  pub filter_end_object: u64,
  pub fill: bool,
  pub fill_start_group: Option<u64>,
  pub switch_after: u64,
  pub switch_track: Option<String>,
  pub switch_mode: SwitchMode,
  pub switch_publish_done: bool,
  pub switch_start_group: u64,
  pub range_filters: Vec<MessageParameter>,
  pub fill_range_filters: Vec<MessageParameter>,
  pub update_filters_after: u64,
  pub update_range_filters: Vec<MessageParameter>,
  pub ignore_filter_limit: bool,
}

pub async fn run(moq: MoqConnection, config: SubscribeConfig) -> Result<()> {
  // Keep `moq` alive for the whole function: its control stream carries only
  // SETUP now, but must stay open for the session's lifetime. Subscriptions use
  // their own bidi request streams; objects arrive on uni streams.
  let connection = moq.connection.clone();

  // Each subscription's request stream is held open for its lifetime.
  let mut request_streams = Vec::new();

  // Which subscription each object belongs to: the alias alone cannot say when a
  // publisher gives two subscriptions to one track the same one.
  let router = Arc::new(Mutex::new(Router::new()));

  // Request IDs a client sends are even.
  let primary_filters = filter_parameters(&config);
  let limit = moq.peer_max_filter_ranges;
  check_limit(&primary_filters, limit, config.ignore_filter_limit)?;
  let mut primary_parameters = primary_filters.clone();
  if !config.include_properties {
    primary_parameters.push(MessageParameter::new_include_properties(false));
  }
  let (track_alias, largest, primary_stream) = subscribe_track(
    &connection,
    &config.namespace,
    &config.track_name,
    0,
    config.subscriber_priority,
    config.group_order,
    config.forward,
    primary_parameters,
    None,
  )
  .await?;
  {
    let mut router = router.lock().unwrap();
    router.add(
      0,
      format!("req=0({})", config.track_name),
      track_alias,
      Window::from_parameters(&primary_filters, largest.as_ref()),
    );
    router.set_filter(0, object_filter(&primary_filters));
  }

  // Optionally send REQUEST_UPDATEs on the subscription's request stream after a
  // delay -- flipping Forward State 0->1, or moving the Range Filters -- then hold
  // the stream open, logging what comes back on it.
  let mut updates = Vec::new();
  if config.update_forward_after > 0 {
    updates.push(ScheduledUpdate {
      after: config.update_forward_after,
      request_id: 10,
      parameters: vec![MessageParameter::new_forward(true)],
    });
  }
  if config.update_filters_after > 0 {
    let parameters = config.update_range_filters.clone();
    let mut resulting = primary_filters.clone();
    apply_message_parameter_update(&mut resulting, parameters.clone());
    check_limit(&parameters, limit, config.ignore_filter_limit)?;
    check_limit(&resulting, limit, config.ignore_filter_limit)?;
    updates.push(ScheduledUpdate {
      after: config.update_filters_after,
      request_id: 12,
      parameters,
    });
  }
  if updates.is_empty() {
    request_streams.push(primary_stream);
  } else {
    send_updates(
      primary_stream,
      updates,
      primary_filters.clone(),
      router.clone(),
    );
  }

  // Switch to another track after a delay: a second SUBSCRIBE carrying SWITCH_FROM,
  // which activates it and suspends the first.
  if config.switch_after > 0
    && let Some(switch_track) = config.switch_track.clone()
  {
    let connection = connection.clone();
    let namespace = config.namespace.clone();
    let delay = config.switch_after;
    let priority = config.subscriber_priority;
    let group_order = config.group_order;
    let mode = config.switch_mode;
    let publish_done = config.switch_publish_done;
    // A start group of its own makes the switch happen at that group rather than
    // at the next one, leaving the suspended track delivering until then, and
    // fills whatever of it is already published.
    let filters = if config.switch_start_group > 0 {
      vec![
        MessageParameter::new_location_filter(LocationFilter::AbsoluteStart {
          start: Location::new(config.switch_start_group, 0),
        }),
        MessageParameter::new_fill_parameters(vec![]),
      ]
    } else {
      filter_parameters(&config)
    };
    let router = router.clone();
    tokio::spawn(async move {
      tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
      info!("Switching to {} ({:?})", switch_track, mode);
      match subscribe_track(
        &connection,
        &namespace,
        &switch_track,
        2,
        priority,
        group_order,
        true,
        filters.clone(),
        Some(MessageParameter::new_switch_from(0, mode, publish_done)),
      )
      .await
      {
        Ok((alias, largest, stream)) => {
          info!("Switched to {} (track_alias={})", switch_track, alias);
          {
            let mut router = router.lock().unwrap();
            router.add(
              2,
              format!("req=2({switch_track})"),
              alias,
              Window::from_parameters(&filters, largest.as_ref()),
            );
            router.set_filter(2, object_filter(&filters));
          }
          // Hold the request stream open for the subscription's lifetime.
          let mut stream = stream;
          loop {
            match stream.next_message().await {
              Ok(m) => info!("switch subscription: {:?}", m),
              Err(e) => {
                error!("switch subscription stream ended: {:?}", e);
                break;
              }
            }
          }
        }
        Err(e) => error!("Switch failed: {:?}", e),
      }
    });
  }

  if let Some((ref extra_name, extra_priority)) = config.extra_track {
    // The extra track is for priority testing and carries no Range Filters.
    let extra_filters = vec![MessageParameter::new_location_filter(
      LocationFilter::NextObject,
    )];
    let (alias, largest, extra_stream) = subscribe_track(
      &connection,
      &config.namespace,
      extra_name,
      4,
      extra_priority,
      config.group_order,
      config.forward,
      extra_filters.clone(),
      None,
    )
    .await?;
    request_streams.push(extra_stream);
    router.lock().unwrap().add(
      4,
      format!("req=4({extra_name})"),
      alias,
      Window::from_parameters(&extra_filters, largest.as_ref()),
    );
  }

  let result = match config.delivery_mode {
    DeliveryMode::Datagram => receive_datagrams(&connection, router, config.duration).await,
    DeliveryMode::Subgroup => receive_streams(&connection, router, config.duration).await,
  };
  drop(request_streams);
  result
}

async fn receive_datagrams(
  connection: &Arc<TransportConnection>,
  router: Arc<Mutex<Router>>,
  duration: u64,
) -> Result<()> {
  info!("Listening for datagrams...");

  let connection_clone = connection.clone();
  let task_router = router.clone();
  let datagram_task = tokio::spawn(async move {
    let router = task_router;
    let mut stats = ReceptionStats::new();

    loop {
      match connection_clone.receive_datagram().await {
        Ok(datagram) => {
          let mut bytes_mut = datagram.clone();

          match Datagram::deserialize(&mut bytes_mut) {
            Ok(obj) => {
              // Sanity check
              if obj.group_id >= 10000 || obj.object_id >= 10000 {
                error!(
                  "Invalid datagram values: group={}, object={}",
                  obj.group_id, obj.object_id
                );
                stats.record_parse_error();
                continue;
              }

              let location = Location::new(obj.group_id, obj.object_id);
              let (route, labels) = route_object(&router, obj.track_alias, &location);
              if route.owners.is_empty() && route.duplicates == 0 {
                debug!(
                  "Ignoring datagram for unknown track_alias: {}",
                  obj.track_alias
                );
                continue;
              }
              stats.record_duplicates(route.duplicates);

              let fields = ObjectFields {
                subgroup_id: None,
                object_id: obj.object_id,
                publisher_priority: obj.publisher_priority.unwrap_or(DEFAULT_PUBLISHER_PRIORITY),
                properties: obj.properties.as_deref(),
              };
              for (owner, label) in route.owners.iter().zip(labels) {
                let status = record(
                  &mut stats,
                  &router,
                  *owner,
                  &location,
                  fields,
                  ReceptionStats::prior_group_gap(obj.properties.as_ref()),
                );
                let sequence_ok = status == "OK";

                if should_log(stats.total_received) || !sequence_ok {
                  info!(
                    "Received datagram {}: subscription={} group={}, object={}, size={} bytes, elapsed={}ms, seq={}",
                    stats.total_received,
                    label,
                    obj.group_id,
                    obj.object_id,
                    obj.payload.as_ref().map_or(0, |p| p.len()),
                    stats.elapsed_ms(),
                    status
                  );
                } else {
                  debug!(
                    "Received datagram {}: subscription={} group={}, object={}, seq=OK",
                    stats.total_received, label, obj.group_id, obj.object_id
                  );
                }
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
  report_subscriptions(&stats, &router);

  Ok(())
}

async fn receive_streams(
  connection: &Arc<TransportConnection>,
  router: Arc<Mutex<Router>>,
  duration: u64,
) -> Result<()> {
  info!("Listening for incoming streams...");

  let pending_fetches = Arc::new(RwLock::new(BTreeMap::new()));
  let conn = connection.clone();
  let pending_fetches_clone = pending_fetches.clone();

  let task_router = router.clone();
  let stream_task = tokio::spawn(async move {
    let router = task_router;
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
                let (route, labels) = route_object(&router, obj.track_alias, &obj.location);
                if route.owners.is_empty() && route.duplicates == 0 {
                  debug!(
                    "Ignoring object for unknown track_alias: {}",
                    obj.track_alias
                  );
                }
                stats.record_duplicates(route.duplicates);

                for (owner, label) in route.owners.iter().zip(labels) {
                  let status = record(
                    &mut stats,
                    &router,
                    *owner,
                    &obj.location,
                    ObjectFields::from(&obj),
                    ReceptionStats::prior_group_gap(obj.properties.as_ref()),
                  );
                  let sequence_ok = status == "OK";

                  if should_log(stats.total_received) || !sequence_ok {
                    info!(
                      "Received object {}: subscription={} group={}, object={}, subgroup={:?}, priority={}, seq={}",
                      stats.total_received,
                      label,
                      obj.location.group,
                      obj.location.object,
                      obj.subgroup_id,
                      obj.publisher_priority,
                      status
                    );
                  } else {
                    debug!(
                      "Received object {}: subscription={} group={}, object={}",
                      stats.total_received, label, obj.location.group, obj.location.object
                    );
                  }
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
  report_subscriptions(&stats, &router);

  Ok(())
}

/// A REQUEST_UPDATE to send some seconds after subscribing.
struct ScheduledUpdate {
  after: u64,
  request_id: u64,
  parameters: Vec<MessageParameter>,
}

/// Sends `updates` on the primary subscription's request stream in time order,
/// moving the Range Filters its objects are checked against as each goes out, and
/// then logs whatever comes back on the stream for as long as it lives.
fn send_updates(
  mut stream: ControlStreamHandler,
  mut updates: Vec<ScheduledUpdate>,
  mut parameters: Vec<MessageParameter>,
  router: Arc<Mutex<Router>>,
) {
  updates.sort_by_key(|u| u.after);
  tokio::spawn(async move {
    let started = tokio::time::Instant::now();
    for update in updates {
      tokio::time::sleep_until(started + tokio::time::Duration::from_secs(update.after)).await;
      info!("Sending REQUEST_UPDATE: {:?}", update.parameters);
      apply_message_parameter_update(&mut parameters, update.parameters.clone());
      let message = RequestUpdate::new(update.request_id, update.parameters);
      if let Err(e) = stream
        .send(&ControlMessage::RequestUpdate(Box::new(message)))
        .await
      {
        error!("Failed to send REQUEST_UPDATE: {:?}", e);
        continue;
      }
      router
        .lock()
        .unwrap()
        .update_filter(0, object_filter(&parameters));
    }
    loop {
      match stream.next_message().await {
        Ok(ControlMessage::RequestError(m)) => error!("Subscription update refused: {:?}", m),
        Ok(m) => info!("Subscription: {:?}", m),
        Err(e) => {
          debug!("Subscription request stream ended: {:?}", e);
          return;
        }
      }
    }
  });
}

/// Counts an object into the subscription that took it: against its Range Filters
/// when it has them, and its sequence otherwise. Returns what to log it as.
fn record(
  stats: &mut ReceptionStats,
  router: &Mutex<Router>,
  owner: u64,
  location: &Location,
  fields: ObjectFields<'_>,
  prior_group_gap: u64,
) -> &'static str {
  let mut router = router.lock().unwrap();
  if router.is_filtered(owner) {
    let passed = router.passes(owner, fields);
    stats.record_filtered_object(owner, location.group, location.object, passed);
    if passed { "OK" } else { "FILTER_VIOLATION" }
  } else if stats.record_object(owner, location.group, location.object, prior_group_gap) {
    "OK"
  } else {
    "GAP"
  }
}

/// Routes an object, and names the subscriptions that took it for the log.
fn route_object(
  router: &Mutex<Router>,
  track_alias: u64,
  location: &Location,
) -> (crate::routing::Route, Vec<String>) {
  let mut router = router.lock().unwrap();
  let route = router.route(track_alias, location);
  let labels = route
    .owners
    .iter()
    .map(|owner| router.label(*owner).to_string())
    .collect();
  (route, labels)
}

/// One line per subscription, then the totals.
fn report_subscriptions(stats: &ReceptionStats, router: &Mutex<Router>) {
  let router = router.lock().unwrap();
  for (request_id, received, gaps, violations) in stats.per_track() {
    info!(
      "Subscription {}: received={}, gaps={}, filter_violations={}",
      router.label(request_id),
      received,
      gaps,
      violations
    );
  }
  info!(
    "Subscriber finished: received={}, errors={}, gaps={}, duplicates={}, filter_violations={}",
    stats.total_received,
    stats.parse_errors,
    stats.sequence_gaps,
    stats.duplicates,
    stats.filter_violations
  );
}
