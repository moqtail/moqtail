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

//! Relay mesh per draft-lcurley-moq-cluster-01.
//!
//! Every namespace the relay can route is a set of routes (who advertised it, the
//! Hop IDs it passed, what it costs). Each cluster peer is told the best route that
//! did not already pass through it, over one advertisement per namespace that is
//! kept in step as routes come and go, and SUBSCRIBE/FETCH/TRACK_STATUS go to the
//! source the same rule picks for the requester.

use crate::server::Server;
use crate::server::session::Session;
use crate::server::session::client::MOQTClient;
use crate::server::session::session_context::SessionContext;
use anyhow::Result;
use moqtail::model::common::pair::KeyValuePair;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::constant::SUPPORTED_VERSIONS;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::namespace::Namespace;
use moqtail::model::control::namespace_done::NamespaceDone;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::request_update::RequestUpdate;
use moqtail::model::control::setup::Setup;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::error::StreamResetCode;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tracing::{info, warn};
use wtransport::quinn;
use wtransport::tls;

/// HOP_ID Setup Option.
pub(crate) const HOP_ID: u64 = 0x40B54;
/// RELAY_COST Setup Option.
pub(crate) const RELAY_COST: u64 = 0x40B56;

const REDIAL_DELAY: Duration = Duration::from_secs(1);

/// Serializes `refresh`, so two of them cannot apply their snapshots out of order and
/// leave a withdrawn route advertised.
static REFRESH: Mutex<()> = Mutex::const_new(());
static NEXT_ADVERT: AtomicU64 = AtomicU64::new(0);

/// What this relay currently advertises to one peer for one namespace.
#[derive(Debug)]
pub(crate) struct Advert {
  /// The session the route came from.
  source: usize,
  hops: Vec<u64>,
  cost: u64,
  sink: Sink,
  generation: u64,
}

#[derive(Debug)]
enum Sink {
  /// Feeds the task holding our PUBLISH_NAMESPACE request stream; dropping it withdraws.
  PublishNamespace(mpsc::UnboundedSender<ControlMessage>),
  /// The peer's SUBSCRIBE_NAMESPACE stream, and the suffix it knows the namespace by.
  Namespace(mpsc::UnboundedSender<ControlMessage>, Tuple),
}

#[derive(Clone)]
struct Route {
  client: Arc<MOQTClient>,
  hops: Vec<u64>,
  /// Accumulated: the received ROUTE_COST plus the sender's RELAY_COST.
  cost: u64,
  /// Higher is more recent.
  seq: usize,
}

/// A varint Setup Option the base protocol does not know.
pub(crate) fn setup_varint(setup: &Setup, key: u64) -> Option<u64> {
  setup.setup_options.iter().find_map(|kvp| match kvp {
    KeyValuePair::VarInt { type_value, value } if *type_value == key => Some(*value),
    _ => None,
  })
}

pub(crate) fn hop_path(parameters: &[MessageParameter]) -> Option<&Vec<u64>> {
  parameters.iter().find_map(|p| match p {
    MessageParameter::HopPath { hops } => Some(hops),
    _ => None,
  })
}

fn route_cost(parameters: &[MessageParameter]) -> u64 {
  parameters
    .iter()
    .find_map(|p| match p {
      MessageParameter::RouteCost { cost } => Some(*cost),
      _ => None,
    })
    .unwrap_or(0)
}

/// Whether a path came back through this relay. 0 identifies nothing, so never loops.
pub(crate) fn loops_through(hops: &[u64], hop_id: u64) -> bool {
  hop_id != 0 && hops.contains(&hop_id)
}

/// Dials a peer for as long as the relay runs, redialing whenever the session ends.
pub(crate) async fn run_peer(server: Server, url: String) {
  loop {
    match dial(&server, &url).await {
      Ok(connection) => {
        info!("Cluster peer {url} connected");
        if let Err(e) = Session::connect_quic(connection.clone(), server.clone()).await {
          warn!("Cluster peer {url} session failed: {e:?}");
        }
        let reason = connection.closed().await;
        info!("Cluster peer {url} disconnected: {reason}");
      }
      Err(e) => warn!("Cluster peer {url} unreachable: {e:#}"),
    }
    tokio::time::sleep(REDIAL_DELAY).await;
  }
}

async fn dial(server: &Server, url: &str) -> Result<quinn::Connection> {
  let config = server.app_config;
  let authority = url.strip_prefix("moqt://").unwrap_or(url);
  let authority = authority.split('/').next().unwrap_or_default();
  let (host, port) = authority
    .rsplit_once(':')
    .ok_or_else(|| anyhow::anyhow!("peer {url} names no port"))?;
  let host = host.trim_start_matches('[').trim_end_matches(']');
  let port: u16 = port.parse()?;

  let mut tls_config = if config.peer_no_cert_validation {
    tls::client::build_default_tls_config(
      Arc::new(tls::rustls::RootCertStore::empty()),
      Some(Arc::new(tls::client::NoServerVerification::new())),
    )
  } else {
    tls::client::build_default_tls_config(Arc::new(tls::build_native_cert_store()), None)
  };
  // Raw QUIC: the ALPN is the MOQT version, which is how the peer's demux tells it
  // from WebTransport.
  tls_config.alpn_protocols = SUPPORTED_VERSIONS
    .split(',')
    .map(|v| v.trim().as_bytes().to_vec())
    .collect();
  let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?;

  let mut transport = quinn::TransportConfig::default();
  transport.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
  transport.keep_alive_interval(Some(config.keep_alive_interval));
  transport.max_idle_timeout(Some(config.max_idle_timeout.try_into()?));
  // The peer opens request streams toward us too.
  transport.max_concurrent_bidi_streams(config.max_request_stream_limit());
  let mut client_config = quinn::ClientConfig::new(Arc::new(crypto));
  client_config.transport_config(Arc::new(transport));

  let addr = tokio::net::lookup_host((host, port))
    .await?
    .next()
    .ok_or_else(|| anyhow::anyhow!("cannot resolve {host}"))?;
  let bind: SocketAddr = if addr.is_ipv6() {
    "[::]:0".parse()?
  } else {
    "0.0.0.0:0".parse()?
  };
  let endpoint = quinn::Endpoint::client(bind)?;
  Ok(endpoint.connect_with(client_config, addr, host)?.await?)
}

/// Every route to exactly `namespace`: announcements, and the namespaces of tracks
/// clients PUBLISH, which are bridged like announcements from them.
async fn routes(context: &SessionContext, namespace: &Tuple) -> Vec<Route> {
  let mut routes = Vec::new();
  let clients: Vec<Arc<MOQTClient>> = {
    let clients = context.client_manager.clients.read().await;
    clients.values().cloned().collect()
  };
  for client in clients {
    // A peer's tracks reach us by announcement, not PUBLISH.
    if client.peer_hop_id.is_some() {
      continue;
    }
    let names: Vec<FullTrackName> = client
      .published_tracks
      .read()
      .await
      .values()
      .filter(|name| &name.namespace == namespace)
      .cloned()
      .collect();
    // An entry outlives its PUBLISH_DONE; the track's publishers say who still serves.
    let mut publishes = false;
    for name in names {
      if let Some(track) = context.track_manager.get_track(&name).await
        && track
          .publisher_aliases
          .read()
          .await
          .contains_key(&client.connection_id)
      {
        publishes = true;
        break;
      }
    }
    if publishes {
      routes.push(Route {
        hops: vec![0],
        cost: client.peer_relay_cost,
        client,
        seq: 0,
      });
    }
  }
  let announcements = context.track_manager.announcement_routes(namespace).await;
  for (seq, (client, message)) in announcements.into_iter().enumerate() {
    // Bridging: an upstream that did not negotiate has an unknown publisher, and
    // anything it put in the message is not this extension's.
    let (hops, cost) = match client.peer_hop_id {
      Some(_) => (
        hop_path(&message.parameters)
          .cloned()
          .unwrap_or_else(|| vec![0]),
        route_cost(&message.parameters),
      ),
      None => (vec![0], 0),
    };
    routes.push(Route {
      hops,
      cost: cost.saturating_add(client.peer_relay_cost),
      client,
      seq: seq + 1,
    });
  }
  routes
}

/// Whether a route may not be given to a peer (§7): it came from that peer's session,
/// or already passed through it. A plain client is never a peer, so it may be served
/// from its own session, as a client subscribing to its own track is.
fn excluded(hops: &[u64], from: usize, to: usize, to_hop: Option<u64>) -> bool {
  // shortcut: a peer that declared 0 is filtered by its session only (§4.2 would
  // give a dialed one a stable assigned ID); fine until anonymous peers multi-link.
  to_hop.is_some_and(|hop| from == to || loops_through(hops, hop))
}

/// Lower is better (§7): no 0 in the path, then lowest cost, shortest path, newest.
fn rank(hops: &[u64], cost: u64, seq: usize) -> (bool, u64, usize, Reverse<usize>) {
  (hops.contains(&0), cost, hops.len(), Reverse(seq))
}

/// The best route `to` may be given.
fn best<'a>(routes: &'a [Route], to: &MOQTClient) -> Option<&'a Route> {
  routes
    .iter()
    .filter(|r| {
      !excluded(
        &r.hops,
        r.client.connection_id,
        to.connection_id,
        to.peer_hop_id,
      )
    })
    .min_by_key(|r| rank(&r.hops, r.cost, r.seq))
}

/// Whether a new path still comes from the publisher an advertisement names, so it is
/// updated in place rather than withdrawn (§6.3). The same non-zero first hop is the
/// same publisher. A 0 names nobody, so it takes the same source session, or the same
/// whole path: that came through the same relays, and a relay sends every session to
/// one peer the same best path, so two links to it never carry different publishers.
fn same_publisher(old: &[u64], old_source: usize, new: &[u64], new_source: usize) -> bool {
  old[0] == new[0] && (new[0] != 0 || old_source == new_source || old == new)
}

/// The best route for `to`, but the one from `current` while it is still as good save
/// for recency. Two links to one peer carry equal routes, and re-advertising one
/// makes it the newest; following that would move the advertisement back and forth
/// between them, withdrawing it each time, around every loop in the mesh.
fn stick<'a>(routes: &'a [Route], to: &MOQTClient, current: Option<usize>) -> Option<&'a Route> {
  let best = best(routes, to)?;
  let level = |r: &Route| {
    let (zero, cost, len, _) = rank(&r.hops, r.cost, r.seq);
    (zero, cost, len)
  };
  let kept = routes.iter().find(|r| {
    Some(r.client.connection_id) == current
      && !excluded(
        &r.hops,
        r.client.connection_id,
        to.connection_id,
        to.peer_hop_id,
      )
      && level(r) == level(best)
  });
  Some(kept.unwrap_or(best))
}

/// Narrows the publishers that match a request to the one source the routing rule
/// picks for `requester`, from the most specific namespace that has any. Unchanged
/// when clustering is off. Empty means the request is unroutable.
pub(crate) async fn select(
  context: &SessionContext,
  requester: &MOQTClient,
  full_track_name: &FullTrackName,
  publishers: Vec<Arc<MOQTClient>>,
) -> Vec<Arc<MOQTClient>> {
  if context.server_config.hop_id.is_none() || publishers.is_empty() {
    return publishers;
  }
  let mut namespace = full_track_name.namespace.clone();
  loop {
    let mut tier = routes(context, &namespace).await;
    tier.retain(|r| {
      publishers
        .iter()
        .any(|p| p.connection_id == r.client.connection_id)
    });
    if !tier.is_empty() {
      return best(&tier, requester)
        .map(|r| vec![r.client.clone()])
        .unwrap_or_default();
    }
    if namespace.fields.pop().is_none() {
      // Matched some other way (an exact track only a peer knows of); leave it be.
      return publishers;
    }
  }
}

/// Whether `source` is serving a track under `namespace` to someone right now, so its
/// ingress is already paid for (§6.2).
async fn carrying(context: &SessionContext, namespace: &Tuple, source: usize) -> bool {
  for track in context.track_manager.tracks_under(namespace).await {
    let from_source = track.publisher_aliases.read().await.contains_key(&source);
    if from_source && track.subscriber_count().await > 0 {
      return true;
    }
  }
  false
}

/// Brings every peer's advertisement of every namespace in line with the routes.
pub(crate) async fn refresh_all(context: Arc<SessionContext>) {
  if context.server_config.hop_id.is_none() {
    return;
  }
  let mut namespaces: HashSet<Tuple> = context
    .track_manager
    .get_announcements_by_prefix(&Tuple::new())
    .await
    .into_iter()
    .collect();
  {
    let clients = context.client_manager.clients.read().await;
    for client in clients.values() {
      namespaces.extend(
        client
          .published_tracks
          .read()
          .await
          .values()
          .map(|name| name.namespace.clone()),
      );
      // Whatever is advertised now, so a namespace that lost its routes is withdrawn.
      namespaces.extend(client.cluster_adverts.lock().await.keys().cloned());
    }
  }
  for namespace in namespaces {
    refresh(&context, &namespace).await;
  }
}

/// Refreshes every namespace that could cover a track in `namespace`, after the track
/// gained or lost the subscribers that make its source warm.
pub(crate) async fn refresh_covering(context: Arc<SessionContext>, mut namespace: Tuple) {
  loop {
    refresh(&context, &namespace).await;
    if namespace.fields.pop().is_none() {
      break;
    }
  }
}

/// Brings every peer's advertisement of `namespace` in line with its routes: opens,
/// updates in place, re-advertises when the publisher changed, or withdraws (§6.3).
pub(crate) async fn refresh(context: &Arc<SessionContext>, namespace: &Tuple) {
  let Some(own_hop) = context.server_config.hop_id else {
    return;
  };
  let _serial = REFRESH.lock().await;
  let routes = routes(context, namespace).await;
  let peers: Vec<Arc<MOQTClient>> = {
    let clients = context.client_manager.clients.read().await;
    clients
      .values()
      .filter(|c| c.peer_hop_id.is_some())
      .cloned()
      .collect()
  };

  for peer in peers {
    let mut adverts = peer.cluster_adverts.lock().await;
    let current = adverts.get(namespace).map(|a| a.source);
    let wanted = match stick(&routes, &peer, current) {
      Some(route) => {
        let mut hops = route.hops.clone();
        hops.push(own_hop);
        let warm = carrying(context, namespace, route.client.connection_id).await;
        Some((
          route.client.connection_id,
          hops,
          if warm { 0 } else { route.cost },
        ))
      }
      None => None,
    };

    let Some((source, hops, cost)) = wanted else {
      if let Some(old) = adverts.remove(namespace) {
        withdraw(old);
      }
      continue;
    };
    let params = vec![
      MessageParameter::HopPath { hops: hops.clone() },
      MessageParameter::RouteCost { cost },
    ];

    if let Some(advert) = adverts.get_mut(namespace) {
      if same_publisher(&advert.hops, advert.source, &hops, source) {
        if advert.hops != hops || advert.cost != cost {
          match &advert.sink {
            Sink::PublishNamespace(tx) => {
              let request_id = peer.next_request_id(&context.relay_next_request_id);
              let _ = tx.send(ControlMessage::RequestUpdate(Box::new(RequestUpdate::new(
                request_id, params,
              ))));
            }
            Sink::Namespace(tx, suffix) => {
              let _ = tx.send(ControlMessage::Namespace(Box::new(
                Namespace::with_parameters(suffix.clone(), params),
              )));
            }
          }
          advert.source = source;
          advert.hops = hops;
          advert.cost = cost;
        }
        continue;
      }
      if let Some(old) = adverts.remove(namespace) {
        withdraw(old);
      }
    }

    let generation = NEXT_ADVERT.fetch_add(1, Ordering::Relaxed);
    let sink = match context
      .track_manager
      .namespace_subscription_of(peer.connection_id, namespace)
      .await
      .and_then(|(prefix, tx)| Some((namespace.suffix(&prefix)?, tx)))
    {
      // The peer asked to discover namespaces; the answer is the advertisement.
      Some((suffix, tx)) => {
        let _ = tx.send(ControlMessage::Namespace(Box::new(
          Namespace::with_parameters(suffix.clone(), params),
        )));
        Sink::Namespace(tx, suffix)
      }
      None => Sink::PublishNamespace(publish_namespace(
        context.clone(),
        peer.clone(),
        namespace.clone(),
        params,
        generation,
      )),
    };
    info!(
      "Advertising {:?} to peer {} with path {:?} cost {}",
      namespace, peer.connection_id, hops, cost
    );
    adverts.insert(
      namespace.clone(),
      Advert {
        source,
        hops,
        cost,
        sink,
        generation,
      },
    );
  }
}

fn withdraw(advert: Advert) {
  // A PUBLISH_NAMESPACE sink is withdrawn by being dropped here.
  if let Sink::Namespace(tx, suffix) = advert.sink {
    let _ = tx.send(ControlMessage::NamespaceDone(Box::new(NamespaceDone::new(
      suffix,
    ))));
  }
}

/// Opens a PUBLISH_NAMESPACE to `peer` and holds its request stream open, sending the
/// updates fed in, until the sender is dropped or the peer ends or refuses it.
fn publish_namespace(
  context: Arc<SessionContext>,
  peer: Arc<MOQTClient>,
  namespace: Tuple,
  params: Vec<MessageParameter>,
  generation: u64,
) -> mpsc::UnboundedSender<ControlMessage> {
  let (tx, mut rx) = mpsc::unbounded_channel();
  tokio::spawn(async move {
    let request_id = peer.next_request_id(&context.relay_next_request_id);
    match peer.connection.open_bi().await {
      Ok((send, recv)) => {
        let mut stream = ControlStreamHandler::new(send, recv).with_peer_id(peer.connection_id);
        let message = PublishNamespace::new(request_id, namespace.clone(), &params);
        if stream
          .send(&ControlMessage::PublishNamespace(Box::new(message)))
          .await
          .is_ok()
        {
          loop {
            tokio::select! {
              update = rx.recv() => match update {
                Some(update) => if stream.send(&update).await.is_err() { break },
                None => break,
              },
              reply = stream.next_message() => match reply {
                Ok(ControlMessage::RequestOk(_)) => continue,
                other => {
                  warn!("Peer {} ended advertisement of {:?}: {:?}", peer.connection_id, namespace, other);
                  break;
                }
              },
            }
          }
        }
        stream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
      }
      Err(e) => warn!(
        "Cannot open PUBLISH_NAMESPACE stream to peer {}: {e:?}",
        peer.connection_id
      ),
    }
    let mut adverts = peer.cluster_adverts.lock().await;
    if adverts
      .get(&namespace)
      .is_some_and(|a| a.generation == generation)
    {
      adverts.remove(&namespace);
    }
  });
  tx
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn zero_never_loops() {
    assert!(!loops_through(&[0, 0, 2], 0));
    assert!(loops_through(&[0, 1, 2], 2));
    assert!(!loops_through(&[0, 1, 2], 3));
  }

  #[test]
  fn exclusion() {
    // A peer never gets its own session's routes, nor one through its Hop ID.
    assert!(excluded(&[0, 1], 7, 7, Some(2)));
    assert!(excluded(&[0, 2, 1], 8, 7, Some(2)));
    assert!(!excluded(&[0, 1], 8, 7, Some(2)));
    // A peer that declared 0 is only kept off its own session.
    assert!(!excluded(&[0, 1], 8, 7, Some(0)));
    assert!(excluded(&[0, 1], 7, 7, Some(0)));
    // A plain client may be served from its own session.
    assert!(!excluded(&[0], 7, 7, None));
  }

  #[test]
  fn publisher_continuity() {
    // A known publisher stays itself whatever the route.
    assert!(same_publisher(&[5, 1], 7, &[5, 2, 1], 8));
    assert!(!same_publisher(&[5, 1], 7, &[6, 1], 7));
    // An unknown one only on the same session, or over the very same path.
    assert!(same_publisher(&[0, 1], 7, &[0, 1, 3], 7));
    assert!(same_publisher(&[0, 1], 7, &[0, 1], 8));
    assert!(!same_publisher(&[0, 1], 7, &[0, 3, 1], 8));
  }

  #[test]
  fn ranking() {
    // (hops, cost, seq), best first: a known publisher beats a 0 one whatever it
    // costs, then cost, then path length, then the newest.
    let ordered = [
      (vec![5, 1], 9, 0),
      (vec![0, 1], 1, 0),
      (vec![0, 1, 2], 2, 3),
      (vec![0, 1, 2, 3], 2, 9),
      (vec![0, 4, 2, 3], 2, 8),
    ];
    for pair in ordered.windows(2) {
      let ((ah, ac, as_), (bh, bc, bs)) = (&pair[0], &pair[1]);
      assert!(rank(ah, *ac, *as_) < rank(bh, *bc, *bs), "{pair:?}");
    }
  }
}
