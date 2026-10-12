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
use crate::routing::{Router, Window};
use crate::stats::ReceptionStats;
use crate::utils::should_log;
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant::{FilterType, GroupOrder, SwitchMode};
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::request_update::RequestUpdate;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::datagram::Datagram;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::transport::connection::TransportConnection;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::RecvDataStream;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// The SUBSCRIPTION_FILTER a `--filter` choice asks for.
fn subscription_filter(config: &SubscribeConfig) -> MessageParameter {
  match config.filter {
    CliFilter::Latest => {
      MessageParameter::new_subscription_filter(FilterType::LatestObject, None, None)
    }
    CliFilter::NextGroup => {
      MessageParameter::new_subscription_filter(FilterType::NextGroupStart, None, None)
    }
    CliFilter::AbsoluteStartFill => MessageParameter::new_subscription_filter(
      FilterType::AbsoluteStartFill,
      Some(Location::new(
        config.filter_start_group,
        config.filter_start_object,
      )),
      None,
    ),
    CliFilter::AbsoluteRangeFill => MessageParameter::new_subscription_filter(
      FilterType::AbsoluteRangeFill,
      Some(Location::new(
        config.filter_start_group,
        config.filter_start_object,
      )),
      Some(config.filter_start_group + config.end_group_delta),
    ),
    CliFilter::RelativeStartFill => {
      MessageParameter::new_relative_start_fill(config.relative_previous)
    }
  }
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
  filter: MessageParameter,
  switch_from: Option<MessageParameter>,
) -> Result<(u64, Option<Location>, ControlStreamHandler)> {
  let ns = Tuple::from_utf8_path(namespace);
  info!(
    "Subscribing to track: {}/{} (request_id={}, priority={}, forward={}, filter={:?})",
    namespace, track_name, request_id, subscriber_priority, forward, filter
  );
  // A switch decides the Forward State itself, so FORWARD must not go with it.
  let mut parameters = vec![
    MessageParameter::new_subscriber_priority(subscriber_priority),
    MessageParameter::new_group_order(group_order),
    filter,
  ];
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
        "Subscribed: track={} track_alias={}",
        track_name, m.track_alias
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
  pub update_forward_after: u64,
  pub filter: CliFilter,
  pub filter_start_group: u64,
  pub filter_start_object: u64,
  pub end_group_delta: u64,
  pub relative_previous: u64,
  pub switch_after: u64,
  pub switch_track: Option<String>,
  pub switch_mode: SwitchMode,
  pub switch_publish_done: bool,
  pub switch_start_group: u64,
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
  let primary_filter = subscription_filter(&config);
  let (track_alias, largest, primary_stream) = subscribe_track(
    &connection,
    &config.namespace,
    &config.track_name,
    0,
    config.subscriber_priority,
    config.group_order,
    config.forward,
    primary_filter.clone(),
    None,
  )
  .await?;
  router.lock().unwrap().add(
    0,
    format!("req=0({})", config.track_name),
    track_alias,
    Window::from_filter(&primary_filter, largest.as_ref()),
  );

  // Optionally flip Forward State 0->1 after a delay by sending a REQUEST_UPDATE
  // on the subscription's request stream, then hold the stream open.
  if config.update_forward_after > 0 {
    let delay = config.update_forward_after;
    let mut stream = primary_stream;
    tokio::spawn(async move {
      tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
      info!("Sending REQUEST_UPDATE to set Forward State 1");
      let update = RequestUpdate::new(10, vec![MessageParameter::new_forward(true)]);
      if let Err(e) = stream
        .send(&ControlMessage::RequestUpdate(Box::new(update)))
        .await
      {
        error!("Failed to send REQUEST_UPDATE: {:?}", e);
      }
      std::future::pending::<()>().await;
    });
  } else {
    request_streams.push(primary_stream);
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
    // at the next one, leaving the suspended track delivering until then.
    let filter = if config.switch_start_group > 0 {
      MessageParameter::new_subscription_filter(
        FilterType::AbsoluteStartFill,
        Some(Location::new(config.switch_start_group, 0)),
        None,
      )
    } else {
      subscription_filter(&config)
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
        filter.clone(),
        Some(MessageParameter::new_switch_from(0, mode, publish_done)),
      )
      .await
      {
        Ok((alias, largest, stream)) => {
          info!("Switched to {} (track_alias={})", switch_track, alias);
          router.lock().unwrap().add(
            2,
            format!("req=2({switch_track})"),
            alias,
            Window::from_filter(&filter, largest.as_ref()),
          );
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
    let extra_filter =
      MessageParameter::new_subscription_filter(FilterType::LatestObject, None, None);
    let (alias, largest, extra_stream) = subscribe_track(
      &connection,
      &config.namespace,
      extra_name,
      4,
      extra_priority,
      config.group_order,
      config.forward,
      extra_filter.clone(),
      None,
    )
    .await?;
    request_streams.push(extra_stream);
    router.lock().unwrap().add(
      4,
      format!("req=4({extra_name})"),
      alias,
      Window::from_filter(&extra_filter, largest.as_ref()),
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

              for (owner, label) in route.owners.iter().zip(labels) {
                let sequence_ok = stats.record_object(
                  *owner,
                  obj.group_id,
                  obj.object_id,
                  ReceptionStats::prior_group_gap(obj.properties.as_ref()),
                );

                if should_log(stats.total_received) || !sequence_ok {
                  info!(
                    "Received datagram {}: subscription={} group={}, object={}, size={} bytes, elapsed={}ms, seq={}",
                    stats.total_received,
                    label,
                    obj.group_id,
                    obj.object_id,
                    obj.payload.as_ref().map_or(0, |p| p.len()),
                    stats.elapsed_ms(),
                    if sequence_ok { "OK" } else { "GAP" }
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
                  let sequence_ok = stats.record_object(
                    *owner,
                    obj.location.group,
                    obj.location.object,
                    ReceptionStats::prior_group_gap(obj.properties.as_ref()),
                  );

                  if should_log(stats.total_received) || !sequence_ok {
                    info!(
                      "Received object {}: subscription={} group={}, object={}, seq={}",
                      stats.total_received,
                      label,
                      obj.location.group,
                      obj.location.object,
                      if sequence_ok { "OK" } else { "GAP" }
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
  for (request_id, received, gaps) in stats.per_track() {
    info!(
      "Subscription {}: received={}, gaps={}",
      router.label(request_id),
      received,
      gaps
    );
  }
  info!(
    "Subscriber finished: received={}, errors={}, gaps={}, duplicates={}",
    stats.total_received, stats.parse_errors, stats.sequence_gaps, stats.duplicates
  );
}
