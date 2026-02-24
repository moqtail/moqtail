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
mod track;
mod track_cache;
mod track_manager;
mod utils;

use crate::server::{config::AppConfig, session::Session};
use anyhow::Result;
use client_manager::ClientManager;
use moqtail::transport::data_stream_handler::{FetchRequest, SubscribeRequest};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::signal;
use tokio::sync::Notify;
use tokio::sync::RwLock;
use tracing::{debug, error, info};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::{EnvFilter, filter::LevelFilter};
use track_manager::TrackManager;
use wtransport::Endpoint;

#[derive(Clone)]
pub(crate) struct Server {
  pub client_manager: Arc<RwLock<ClientManager>>,
  pub track_manager: TrackManager,
  pub relay_fetch_requests: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
  pub relay_subscribe_requests: Arc<RwLock<BTreeMap<u64, SubscribeRequest>>>,
  pub relay_track_status_requests: Arc<RwLock<BTreeMap<u64, SubscribeRequest>>>,
  pub app_config: &'static AppConfig,
  pub relay_next_request_id: Arc<AtomicU64>,
}

impl Server {
  pub fn new() -> Self {
    let config = AppConfig::load();

    init_logging(&config.log_folder);

    debug!("Server | App. Config.: {:?}", config);

    Server {
      client_manager: Arc::new(RwLock::new(ClientManager::new())),
      track_manager: TrackManager::new(),
      relay_fetch_requests: Arc::new(RwLock::new(BTreeMap::new())),
      relay_subscribe_requests: Arc::new(RwLock::new(BTreeMap::new())),
      relay_track_status_requests: Arc::new(RwLock::new(BTreeMap::new())),
      app_config: config,
      relay_next_request_id: Arc::new(AtomicU64::new(1u64)), // relay's request id starts at 1 and are odd
    }
  }

  pub async fn start(&mut self) -> Result<()> {
    let server_config = self.app_config.build_server_config().await?;
    let server = match Endpoint::server(server_config) {
      Ok(server) => server,
      Err(e) => {
        error!("Failed to create server endpoint. Error: {:?}", e);
        return Err(anyhow::Error::from(e));
      }
    };

    info!(
      "{} is running at https://{}:{}",
      env!("MOQTAIL_VERSION"),
      self.app_config.host,
      self.app_config.port
    );

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
        incoming_session = server.accept() => {
          let server = self.clone();
          tokio::spawn(async move {
            match Session::new(incoming_session, server).await {
              Ok(_) => {
                info!("New session: {}", id);
              }
              Err(e) => {
                error!("Error occurred in session {}: {:?}", id, e);
              }
            }
          });
          session_id_counter += 1;
        }
      }
    }
    Ok(())
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
