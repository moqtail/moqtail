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

mod cluster;
mod config;
pub(crate) mod inspect;
mod logging;
mod message_handlers;
mod requests;
mod session;
mod ssts;
mod stream_id;
mod subscription;
mod track;
mod utils;

use crate::server::requests::UpstreamRequests;
use crate::server::session::client::MOQTClient;
use crate::server::{config::AppConfig, session::Session};
use anyhow::Result;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::goaway::GoAway;
use moqtail::transport::qlog;
use session::client_manager::ClientManager;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::signal;
use tokio::sync::{RwLock, watch};
use tracing::{error, info, trace, warn};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};
use track::track_manager::TrackManager;
use wtransport::endpoint::IncomingSessionFuture;
use wtransport::quinn;

const CACHE_ONLY_TRACK_SWEEP_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(crate) struct Server {
  pub client_manager: ClientManager,
  pub track_manager: TrackManager,
  pub upstream_requests: UpstreamRequests,
  pub app_config: &'static AppConfig,
  pub relay_next_request_id: Arc<AtomicU64>,
  /// Request streams currently being served across the relay (for load shedding).
  pub active_request_streams: Arc<AtomicU64>,
  /// Set once the relay is draining (GOAWAY broadcast); new requests are rejected.
  pub draining: Arc<AtomicBool>,
  /// Set once the QUIC listeners accept connections.
  pub accepting: Arc<AtomicBool>,
  /// When the relay started; reported by the observability layer.
  #[cfg_attr(not(feature = "observability"), allow(dead_code))]
  pub started_at: std::time::SystemTime,
  pub cluster: Arc<cluster::ClusterState>,
  /// GOAWAY redirect URI clients should migrate to. Seeded from config; the API's
  /// `actions` feature can change it at runtime.
  pub redirect_uri: Arc<RwLock<Option<String>>>,
}

impl Server {
  pub fn new() -> Self {
    let config = AppConfig::load();

    init_logging(config);

    trace!("Server | App. Config.: {:?}", config);

    Server {
      client_manager: ClientManager::new(),
      track_manager: TrackManager::new(),
      upstream_requests: UpstreamRequests::default(),
      app_config: config,
      relay_next_request_id: Arc::new(AtomicU64::new(0)), // see MOQTClient::next_request_id
      active_request_streams: Arc::new(AtomicU64::new(0)),
      draining: Arc::new(AtomicBool::new(false)),
      accepting: Arc::new(AtomicBool::new(false)),
      started_at: std::time::SystemTime::now(),
      cluster: Arc::default(),
      redirect_uri: Arc::new(RwLock::new(config.redirect_uri.clone())),
    }
  }

  /// The redirect URI advertised in GOAWAY. Reads the runtime value; the config
  /// default seeds it at startup.
  async fn redirect_uri(&self) -> Option<String> {
    self.redirect_uri.read().await.clone()
  }

  pub async fn start(&mut self) -> Result<()> {
    let (endpoints, quic_server_config) = self.app_config.build_quic_endpoints().await?;

    let bind = self.app_config.bind;
    // A wildcard address is not something a client can dial.
    let addr = if bind.ip().is_unspecified() {
      format!("localhost:{}", bind.port())
    } else {
      bind.to_string()
    };
    info!(
      "{} is running on {} UDP socket(s) -- WebTransport clients: https://{addr}  Raw QUIC clients: moqt://{addr}",
      env!("MOQTAIL_VERSION"),
      endpoints.len(),
    );

    // Level-triggered, so an accept loop between two iterations when shutdown is
    // signalled still sees it, which a Notify would not guarantee.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    // Only used to correlate log lines, so any unique value will do.
    let session_id_counter = Arc::new(AtomicU64::new(0));

    // A cache-only track stays as long as it caches something, so the ones left holding
    // nothing are swept up now and then.
    let track_manager = self.track_manager.clone();
    let mut shutdown = shutdown_rx.clone();
    tokio::spawn(async move {
      let mut sweep = tokio::time::interval(CACHE_ONLY_TRACK_SWEEP_INTERVAL);
      loop {
        tokio::select! {
          _ = shutdown.changed() => break,
          _ = sweep.tick() => {
            let removed = track_manager.sweep_empty_cache_only_tracks().await;
            if removed > 0 {
              info!("Removed {removed} cache-only tracks holding nothing");
            }
          }
        }
      }
    });

    for url in &self.app_config.peers {
      tokio::spawn(cluster::run_peer(self.clone(), url.clone()));
    }

    let mut accept_loops = Vec::with_capacity(endpoints.len());
    for (socket_index, endpoint) in endpoints.into_iter().enumerate() {
      let server = self.clone();
      let quic_server_config = quic_server_config.clone();
      let counter = session_id_counter.clone();
      let mut shutdown = shutdown_rx.clone();
      accept_loops.push(tokio::spawn(async move {
        loop {
          tokio::select! {
            _ = shutdown.changed() => break,
            incoming = endpoint.accept() => {
              let Some(incoming) = incoming else {
                info!("QUIC endpoint on socket {} closed, exiting accept loop", socket_index);
                break;
              };
              let id = counter.fetch_add(1, Ordering::Relaxed);
              let server = server.clone();
              let quic_server_config = quic_server_config.clone();
              tokio::spawn(async move {
                match Self::accept_incoming(incoming, server, &quic_server_config).await {
                  Ok(_) => {
                    info!("New session: {}", id);
                  }
                  Err(e) => {
                    error!("Error occurred in session {}: {:?}", id, e);
                    relay_hooks::reporter().connection_failed("handshake");
                  }
                }
              });
            }
          }
        }
      }));
    }

    self.accepting.store(true, Ordering::Relaxed);

    // SIGTERM drains the relay: broadcast GOAWAY, reject new requests, then exit
    // after the drain timeout.
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
      .expect("Failed to install SIGTERM handler");
    const DRAIN_TIMEOUT_MS: u64 = 10_000;

    tokio::select! {
      _ = signal::ctrl_c() => {
        info!("Ctrl-C received, exiting...");
      },
      _ = sigterm.recv() => {
        info!("SIGTERM received, draining (GOAWAY, timeout {}ms)...", DRAIN_TIMEOUT_MS);
        self.draining.store(true, Ordering::Relaxed);
        self.broadcast_goaway(DRAIN_TIMEOUT_MS).await;
        // Keep accepting while draining; a second Ctrl-C cuts it short.
        tokio::select! {
          _ = tokio::time::sleep(Duration::from_millis(DRAIN_TIMEOUT_MS)) => {},
          _ = signal::ctrl_c() => info!("Ctrl-C received during drain, exiting..."),
        }
      },
    }

    let _ = shutdown_tx.send(true);
    for accept_loop in accept_loops {
      let _ = accept_loop.await;
    }
    Ok(())
  }

  /// Sends GOAWAY to every connected client so they can migrate before the relay exits.
  /// Returns how many clients were told.
  async fn broadcast_goaway(&self, timeout_ms: u64) -> usize {
    let redirect_uri = self.redirect_uri().await;
    let clients: Vec<Arc<MOQTClient>> = {
      let clients = self.client_manager.clients.read().await;
      clients.values().cloned().collect()
    };
    info!(
      "Broadcasting GOAWAY (uri: {:?}) to {} client(s)",
      redirect_uri,
      clients.len()
    );
    for client in &clients {
      let goaway = GoAway::new(redirect_uri.clone(), timeout_ms);
      client
        .queue_message(ControlMessage::Goaway(Box::new(goaway)))
        .await;
    }
    clients.len()
  }

  /// Demultiplexes one incoming QUIC connection by its negotiated ALPN protocol —
  /// `h3` is routed to the existing WebTransport path, any MOQT version string
  /// (e.g. `moqt-18`) is routed to the raw-QUIC path. Both share the same port; see
  /// `AppConfig::build_quic_endpoints` for the listener setup.
  async fn accept_incoming(
    incoming: quinn::Incoming,
    server: Server,
    quic_server_config: &quinn::ServerConfig,
  ) -> Result<()> {
    let remote_addr = incoming.remote_address();
    // Debug builds trace this connection's QUIC packets when qlog is on.
    #[cfg(debug_assertions)]
    let own_config = quic_qlog_server_config(&server, quic_server_config, remote_addr);
    #[cfg(not(debug_assertions))]
    let own_config: Option<Arc<quinn::ServerConfig>> = {
      let _ = quic_server_config;
      None
    };
    let mut connecting = match own_config {
      Some(config) => incoming.accept_with(config)?,
      None => incoming.accept()?,
    };

    let handshake_data = connecting.handshake_data().await?;
    let protocol = handshake_data
      .downcast::<quinn::crypto::rustls::HandshakeData>()
      .ok()
      .and_then(|data| data.protocol);

    match protocol.as_deref() {
      Some(b"h3") => {
        trace!(remote = %remote_addr, "demuxed incoming QUIC connection as WebTransport (h3)");
        let session_request = IncomingSessionFuture::with_quic_connecting(connecting).await?;
        Session::accept_webtransport(session_request, server)
          .await
          .map(|_| ())
      }
      Some(other) => {
        trace!(
          remote = %remote_addr,
          alpn = %String::from_utf8_lossy(other),
          "demuxed incoming QUIC connection as raw QUIC"
        );
        let connection = connecting.await?;
        Session::accept_quic(connection, server).await.map(|_| ())
      }
      None => {
        warn!(
          remote = %remote_addr,
          "QUIC connection completed handshake with no negotiated ALPN protocol; rejecting"
        );
        Err(anyhow::anyhow!("no ALPN protocol negotiated"))
      }
    }
  }
}

/// A server config whose transport writes a QUIC qlog of this connection into the
/// qlog folder. `None` when qlog is off or the file cannot be made. Packet events are
/// written as quinn handles each packet, which is why only debug builds do this.
#[cfg(debug_assertions)]
fn quic_qlog_server_config(
  server: &Server,
  base: &quinn::ServerConfig,
  remote: std::net::SocketAddr,
) -> Option<Arc<quinn::ServerConfig>> {
  let dir = server.app_config.qlog_folder.as_ref()?;
  let started = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis())
    .unwrap_or(0);
  let remote_name = remote.to_string().replace([':', '[', ']'], "_");
  let path = std::path::Path::new(dir).join(format!("server_{started}_{remote_name}_quic.sqlog"));
  let file = match std::fs::File::create(&path) {
    Ok(file) => file,
    Err(e) => {
      warn!("Cannot create QUIC qlog {}: {e}", path.display());
      return None;
    }
  };
  let mut qlog = quinn::QlogConfig::default();
  qlog
    .writer(Box::new(std::io::BufWriter::new(file)))
    .title(Some(format!("moqtail relay, remote {remote}")));
  let mut transport = server.app_config.transport_config().ok()?;
  transport.qlog_stream(qlog.into_stream());
  let mut config = base.clone();
  config.transport_config(Arc::new(transport));
  Some(Arc::new(config))
}

fn init_logging(config: &AppConfig) {
  let env_filter = EnvFilter::builder()
    .with_default_directive(LevelFilter::INFO.into())
    .from_env_lossy();

  let builder = tracing_subscriber::fmt()
    .with_target(true)
    .with_level(true)
    .with_env_filter(env_filter);
  // The filter can be changed at runtime only when the API's `actions` feature is
  // on. A changeable filter takes a read lock each time it checks or writes a log
  // event, so relays without the feature keep the plain one.
  #[cfg(feature = "observability")]
  if config.api_enable
    && config
      .api_features
      .as_ref()
      .is_some_and(|f| f.iter().any(|f| f == "actions"))
  {
    let builder = builder.with_filter_reloading();
    inspect::keep_log_filter(builder.reload_handle());
    builder.init();
  } else {
    builder.init();
  }
  #[cfg(not(feature = "observability"))]
  builder.init();

  if let Some(dir) = &config.qlog_folder {
    match qlog::init(dir, "server") {
      Ok(()) => {
        info!("Writing qlog traces to {dir}");
        #[cfg(debug_assertions)]
        info!("Debug build: also writing QUIC packet qlog per connection to {dir}");
      }
      Err(e) => error!("Cannot write qlog traces to {dir}: {e}"),
    }
  }
}
