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

//! Relay mesh.
//!
//! Every namespace the relay can route is a set of routes (who advertised it, the
//! Hop IDs it passed, what it costs). Each cluster peer is told the best route that
//! did not already pass through it, over one advertisement per namespace that is
//! kept in step as routes come and go, and SUBSCRIBE/FETCH/TRACK_STATUS go to the
//! source the same rule picks for the requester.
//!
//! Tracks a client PUBLISHes are also pushed to peers whose own clients asked for
//! them with SUBSCRIBE_TRACKS: the relay subscribes each peer to the prefixes its
//! clients hold, and the peer answers with a PUBLISH for every track its own clients
//! publish under them.

use crate::server::Server;
use crate::server::session::Session;
use crate::server::session::client::MOQTClient;
use crate::server::session::session_context::SessionContext;
use anyhow::Result;
use moqtail::model::common::pair::KeyValuePair;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::constant::SUPPORTED_VERSIONS;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::request_update::RequestUpdate;
use moqtail::model::control::setup::Setup;
use moqtail::model::control::subscribe_tracks::SubscribeTracks;
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

/// Relay-wide clustering state, shared by every session.
#[derive(Debug, Default)]
pub(crate) struct ClusterState {
  /// Serializes `refresh`, so two of them cannot apply their snapshots out of order
  /// and leave a withdrawn route advertised.
  refresh: Mutex<()>,
  /// Serializes `sync_tracks` the same way.
  tracks: Mutex<()>,
  next_advert: AtomicU64,
}

/// A SUBSCRIBE_TRACKS this relay holds on a peer; dropping it cancels the request.
#[derive(Debug)]
pub(crate) struct TracksRequest {
  _cancel: mpsc::UnboundedSender<()>,
  generation: u64,
}

/// What this relay currently advertises to one peer for one namespace.
#[derive(Debug)]
pub(crate) struct Advert {
  /// The session the route came from.
  source: usize,
  hops: Vec<u64>,
  cost: u64,
  /// Feeds the task holding the PUBLISH_NAMESPACE request stream; dropping it
  /// withdraws the advertisement.
  updates: mpsc::UnboundedSender<ControlMessage>,
  generation: u64,
}

impl Advert {
  /// The session the route came from, the hops it took, its cost and generation,
  /// for the observability API.
  #[cfg(feature = "observability")]
  pub(crate) fn describe(&self) -> (usize, &[u64], u64, u64) {
    (self.source, &self.hops, self.cost, self.generation)
  }
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
  let rest = url.strip_prefix("moqt://").unwrap_or(&url);
  let rest = rest.split('#').next().unwrap_or_default();
  let (authority, path) = match rest.find('/') {
    Some(i) => rest.split_at(i),
    None => (rest, ""),
  };
  loop {
    match dial(&server, authority).await {
      Ok(connection) => {
        info!("Cluster peer {url} connected");
        let session = Session::connect_quic(
          connection.clone(),
          server.clone(),
          authority.into(),
          path.into(),
        );
        if let Err(e) = session.await {
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

async fn dial(server: &Server, authority: &str) -> Result<quinn::Connection> {
  let config = server.app_config;
  let (host, port) = authority
    .rsplit_once(':')
    .ok_or_else(|| anyhow::anyhow!("peer {authority} names no port"))?;
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
  for connection_id in context
    .track_manager
    .publishers_of_namespace(namespace)
    .await
  {
    // A peer's tracks reach us by announcement, not PUBLISH.
    match context.client_manager.get(connection_id).await {
      Some(client) if client.peer_hop_id.is_none() => routes.push(Route {
        hops: vec![0],
        cost: client.peer_relay_cost,
        client,
        seq: 0,
      }),
      _ => {}
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

/// Whether a route may not be given to a peer: it came from that peer's session,
/// or already passed through it. A plain client is never a peer, so it may be served
/// from its own session, as a client subscribing to its own track is.
fn excluded(hops: &[u64], from: usize, to: usize, to_hop: Option<u64>) -> bool {
  // shortcut: a peer that declared 0 is filtered by its session only (a dialed one
  // could get a stable assigned ID instead); fine until anonymous peers multi-link.
  to_hop.is_some_and(|hop| from == to || loops_through(hops, hop))
}

/// Lower is better: no 0 in the path, then lowest cost, shortest path, newest.
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
/// updated in place rather than withdrawn. The same non-zero first hop is the
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
/// ingress is already paid for and it is advertised at cost 0.
// shortcut: two warm relays would each see the other's 0 as cheaper than their own
// source. That is safe only because `select` runs once per request and never moves a
// live subscription; re-parenting one needs a deterministic tie-break first.
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
  namespaces.extend(context.track_manager.published_namespaces().await);
  {
    // Whatever is advertised now, so a namespace that lost its routes is withdrawn.
    let clients = context.client_manager.clients.read().await;
    for client in clients.values() {
      namespaces.extend(client.cluster_adverts.lock().await.keys().cloned());
    }
  }
  for namespace in namespaces {
    refresh(&context, &namespace).await;
  }
}

/// After a session closed. A peer may have carried any route, so everything is redone.
/// Anyone else only changes what it announced or published, and the tracks it left
/// without a subscriber.
pub(crate) async fn refresh_after_close(
  context: &Arc<SessionContext>,
  cleanup_tracks: &[FullTrackName],
) {
  if context.server_config.hop_id.is_none() {
    return;
  }
  let Some(client) = context.get_client().await else {
    return;
  };
  if client.peer_hop_id.is_some() {
    refresh_all(context.clone()).await;
    return;
  }
  sync_tracks(context.clone()).await;
  let mut sourced: HashSet<Tuple> = client
    .announced_track_namespaces
    .read()
    .await
    .iter()
    .cloned()
    .collect();
  sourced.extend(
    client
      .published_tracks
      .read()
      .await
      .values()
      .map(|name| name.namespace.clone()),
  );
  for namespace in &sourced {
    refresh(context, namespace).await;
  }
  let mut cooled = HashSet::new();
  for name in cleanup_tracks {
    let idle = match context.track_manager.get_track(name).await {
      Some(track) => track.subscriber_count().await == 0,
      None => true,
    };
    if idle && cooled.insert(name.namespace.clone()) {
      refresh_covering(context.clone(), name.namespace.clone()).await;
    }
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
/// updates in place, re-advertises when the publisher changed, or withdraws.
pub(crate) async fn refresh(context: &Arc<SessionContext>, namespace: &Tuple) {
  let Some(own_hop) = context.server_config.hop_id else {
    return;
  };
  let _serial = context.cluster.refresh.lock().await;
  let peers: Vec<Arc<MOQTClient>> = {
    let clients = context.client_manager.clients.read().await;
    clients
      .values()
      .filter(|c| c.peer_hop_id.is_some())
      .cloned()
      .collect()
  };
  if peers.is_empty() {
    return;
  }
  let routes = routes(context, namespace).await;

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
      adverts.remove(namespace);
      continue;
    };
    let params = vec![
      MessageParameter::HopPath { hops: hops.clone() },
      MessageParameter::RouteCost { cost },
    ];

    if let Some(advert) = adverts.get_mut(namespace) {
      if same_publisher(&advert.hops, advert.source, &hops, source) {
        if advert.hops != hops || advert.cost != cost {
          let request_id = peer.next_request_id(&context.relay_next_request_id);
          let _ = advert
            .updates
            .send(ControlMessage::RequestUpdate(Box::new(RequestUpdate::new(
              request_id, params,
            ))));
          advert.source = source;
          advert.hops = hops;
          advert.cost = cost;
        }
        continue;
      }
      // A new publisher is withdrawn and advertised afresh.
      adverts.remove(namespace);
    }

    let generation = context.cluster.next_advert.fetch_add(1, Ordering::Relaxed);
    let updates = publish_namespace(
      context.clone(),
      peer.clone(),
      namespace.clone(),
      params,
      generation,
    );
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
        updates,
        generation,
      },
    );
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

/// Whether `track` may be pushed to a SUBSCRIBE_TRACKS `subscriber`. A peer gets only
/// the tracks this relay's own clients publish; it asks every other peer itself.
// shortcut: reaches only directly peered relays, so a mesh must be full for players on
// any two relays to find each other; a ring needs these PUBLISHes forwarded on with a
// HOP_PATH, like the namespace adverts.
pub(crate) async fn pushes_to(
  context: &SessionContext,
  track: &crate::server::track::Track,
  subscriber: &MOQTClient,
) -> bool {
  if subscriber.peer_hop_id.is_none() {
    return true;
  }
  let publishers: Vec<usize> = track
    .publisher_aliases
    .read()
    .await
    .keys()
    .copied()
    .collect();
  for id in publishers {
    if let Some(publisher) = context.client_manager.get(id).await
      && publisher.peer_hop_id.is_none()
    {
      return true;
    }
  }
  false
}

/// The prefixes to ask peers for: each one held by a client, without those another
/// already covers, which a peer would refuse as overlapping.
fn outermost(mut prefixes: Vec<Tuple>) -> Vec<Tuple> {
  prefixes.sort_by_key(|p| p.fields.len());
  let mut kept: Vec<Tuple> = Vec::new();
  for prefix in prefixes {
    if !kept.iter().any(|k| prefix.starts_with(k)) {
      kept.push(prefix);
    }
  }
  kept
}

/// Holds a SUBSCRIBE_TRACKS on every peer for each prefix this relay's own clients
/// hold, and cancels the ones no client holds any more.
// shortcut: a cancelled request leaves the tracks already pushed here flowing until
// their publishers end them; ending those needs PUBLISH_DONE per pushed track.
pub(crate) async fn sync_tracks(context: Arc<SessionContext>) {
  if context.server_config.hop_id.is_none() {
    return;
  }
  let _serial = context.cluster.tracks.lock().await;
  let wanted = outermost(context.track_manager.client_tracks_prefixes().await);
  let peers: Vec<Arc<MOQTClient>> = {
    let clients = context.client_manager.clients.read().await;
    clients
      .values()
      .filter(|c| c.peer_hop_id.is_some())
      .cloned()
      .collect()
  };
  for peer in peers {
    let mut held = peer.cluster_tracks.lock().await;
    held.retain(|prefix, _| wanted.contains(prefix));
    for prefix in &wanted {
      if !held.contains_key(prefix) {
        let generation = context.cluster.next_advert.fetch_add(1, Ordering::Relaxed);
        info!(
          "Asking peer {} for tracks under {:?}",
          peer.connection_id, prefix
        );
        let cancel = subscribe_tracks(context.clone(), peer.clone(), prefix.clone(), generation);
        held.insert(
          prefix.clone(),
          TracksRequest {
            _cancel: cancel,
            generation,
          },
        );
      }
    }
  }
}

/// Opens a SUBSCRIBE_TRACKS on `peer` and holds its request stream until the sender
/// is dropped or the peer ends or refuses it. The PUBLISHes it brings arrive on their
/// own streams, as any client's do.
fn subscribe_tracks(
  context: Arc<SessionContext>,
  peer: Arc<MOQTClient>,
  prefix: Tuple,
  generation: u64,
) -> mpsc::UnboundedSender<()> {
  let (tx, mut rx) = mpsc::unbounded_channel::<()>();
  tokio::spawn(async move {
    let request_id = peer.next_request_id(&context.relay_next_request_id);
    match peer.connection.open_bi().await {
      Ok((send, recv)) => {
        let mut stream = ControlStreamHandler::new(send, recv).with_peer_id(peer.connection_id);
        let message = SubscribeTracks::new(request_id, prefix.clone(), vec![]);
        if stream
          .send(&ControlMessage::SubscribeTracks(Box::new(message)))
          .await
          .is_ok()
        {
          loop {
            tokio::select! {
              _ = rx.recv() => break,
              reply = stream.next_message() => match reply {
                Ok(ControlMessage::RequestOk(_)) => continue,
                other => {
                  warn!("Peer {} ended SUBSCRIBE_TRACKS {:?}: {:?}", peer.connection_id, prefix, other);
                  break;
                }
              },
            }
          }
        }
        stream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
      }
      Err(e) => warn!(
        "Cannot open SUBSCRIBE_TRACKS stream to peer {}: {e:?}",
        peer.connection_id
      ),
    }
    let mut held = peer.cluster_tracks.lock().await;
    if held
      .get(&prefix)
      .is_some_and(|r| r.generation == generation)
    {
      held.remove(&prefix);
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
  fn outermost_prefixes() {
    let t = |fields: &[&str]| Tuple::from_utf8_path(&fields.join("/"));
    let mut got = outermost(vec![
      t(&["a", "b"]),
      t(&["a"]),
      t(&["c", "d"]),
      t(&["a", "x", "y"]),
    ]);
    got.sort_by_key(|p| p.fields.len());
    assert_eq!(got, vec![t(&["a"]), t(&["c", "d"])]);
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
