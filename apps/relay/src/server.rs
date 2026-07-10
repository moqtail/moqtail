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

mod client;
mod client_manager;
mod config;
mod errors;
mod message_handlers;
mod object_logger;
mod session;
mod session_context;
mod stream_id;
mod subscription;
mod subscription_manager;
mod token_logger;
mod top_n_coordinator;
mod track;
mod track_cache;
mod track_manager;
mod utils;

use crate::server::session_context::{PendingRequest, UpstreamFetchEvent};
use crate::server::top_n_coordinator::TopNCoordinator;
use crate::server::{config::AppConfig, session::Session};
use anyhow::Result;
use client_manager::ClientManager;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::signal;
use tokio::sync::{Notify, RwLock, mpsc};
use tracing::{debug, error, info, warn};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::{EnvFilter, filter::LevelFilter};
use track_manager::TrackManager;
use wtransport::endpoint::IncomingSessionFuture;
use wtransport::quinn;

#[derive(Clone)]
pub(crate) struct Server {
  pub client_manager: Arc<RwLock<ClientManager>>,
  pub track_manager: TrackManager,
  pub relay_pending_requests: Arc<RwLock<BTreeMap<u64, PendingRequest>>>,
  pub app_config: &'static AppConfig,
  pub relay_next_request_id: Arc<AtomicU64>,
  pub upstream_fetch_senders: Arc<RwLock<BTreeMap<u64, mpsc::Sender<UpstreamFetchEvent>>>>,
  pub top_n_coordinator: Arc<TopNCoordinator>,
}

impl Server {
  pub fn new() -> Self {
    let config = AppConfig::load();

    init_logging(&config.log_folder);

    debug!("Server | App. Config.: {:?}", config);

    let relay_next_request_id = Arc::new(AtomicU64::new(1u64));
    let relay_pending_requests = Arc::new(RwLock::new(BTreeMap::new()));
    let track_manager = TrackManager::new();

    let top_n_coordinator = Arc::new(TopNCoordinator::new(
      track_manager.clone(),
      relay_next_request_id.clone(),
      relay_pending_requests.clone(),
      config,
    ));

    Server {
      client_manager: Arc::new(RwLock::new(ClientManager::new())),
      track_manager,
      relay_pending_requests,
      app_config: config,
      relay_next_request_id,
      upstream_fetch_senders: Arc::new(RwLock::new(BTreeMap::new())),
      top_n_coordinator,
    }
  }

  pub async fn start(&mut self) -> Result<()> {
    let endpoint = self.app_config.build_quic_endpoint().await?;

    info!(
      "{} is running -- WebTransport clients: https://{}:{}  Raw QUIC clients: moqt://{}:{}",
      env!("MOQTAIL_VERSION"),
      self.app_config.host,
      self.app_config.port,
      self.app_config.host,
      self.app_config.port
    );

    // Start the Top-N coordinator tick loop.
    self.top_n_coordinator.clone().spawn_tick_loop();

    let shutdown_notify = Arc::new(Notify::new());
    let notify_clone = shutdown_notify.clone();
    // catch ctrl-c signal to shutdown the server gracefully
    let _ctrl_c_handler = tokio::spawn(async move {
      signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl-c signal");
      notify_clone.notify_waiters();
    });

    let notify_clone = shutdown_notify.clone();

    let mut session_id_counter: u64 = 0;

    loop {
      let id = session_id_counter;
      tokio::select! {
        _ = notify_clone.notified() => {
          info!("Ctrl-C received, exiting...");
          break;
        },
        incoming = endpoint.accept() => {
          let Some(incoming) = incoming else {
            info!("QUIC endpoint closed, exiting accept loop");
            break;
          };
          let server = self.clone();
          tokio::spawn(async move {
            match Self::accept_incoming(incoming, server).await {
              Ok(_) => {
                info!("New session: {}", id);
              }
              Err(e) => {
                error!("Error occurred in session {}: {:?}", id, e);
              }
            }
          });
          if session_id_counter == u64::MAX {
            session_id_counter = 0;
          } else {
            session_id_counter += 1;
          }
        }
      }
    }
    Ok(())
  }

  /// Demultiplexes one incoming QUIC connection by its negotiated ALPN protocol —
  /// `h3` is routed to the existing WebTransport path, any MOQT version string
  /// (e.g. `moqt-16`) is routed to the raw-QUIC path. Both share the same UDP
  /// socket/port; see `AppConfig::build_quic_endpoint` for the listener setup.
  async fn accept_incoming(incoming: quinn::Incoming, server: Server) -> Result<()> {
    let remote_addr = incoming.remote_address();
    let mut connecting = incoming.accept()?;

    let handshake_data = connecting.handshake_data().await?;
    let protocol = handshake_data
      .downcast::<quinn::crypto::rustls::HandshakeData>()
      .ok()
      .and_then(|data| data.protocol);

    match protocol.as_deref() {
      Some(b"h3") => {
        debug!(remote = %remote_addr, "demuxed incoming QUIC connection as WebTransport (h3)");
        let session_request = IncomingSessionFuture::with_quic_connecting(connecting).await?;
        Session::accept_webtransport(session_request, server)
          .await
          .map(|_| ())
      }
      Some(other) => {
        debug!(
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

fn init_logging(log_dir: &str) {
  let env_filter = EnvFilter::builder()
    .with_default_directive(LevelFilter::INFO.into())
    .from_env_lossy();

  // Ensure the log directory exists
  std::fs::create_dir_all(log_dir).expect("Failed to create log directory");

  let file_appender = tracing_appender::rolling::daily(log_dir, "relay.log");
  let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

  tracing_subscriber::fmt()
    .with_target(true)
    .with_level(true)
    .with_env_filter(env_filter)
    .with_writer(non_blocking.and(std::io::stdout))
    .init();
}
