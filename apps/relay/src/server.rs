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
mod track;
mod track_cache;
mod utils;

use crate::server::{config::AppConfig, session::Session};
use anyhow::Result;
use client_manager::ClientManager;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::transport::data_stream_handler::{FetchRequest, SubscribeRequest};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::{EnvFilter, filter::LevelFilter};
use track::Track;
use wtransport::Endpoint;

#[derive(Clone)]
pub(crate) struct Server {
  pub client_manager: Arc<RwLock<ClientManager>>,
  pub tracks: Arc<RwLock<HashMap<FullTrackName, Track>>>, // the tracks the relay is subscribed to, key is the track alias
  pub track_aliases: Arc<RwLock<BTreeMap<u64, FullTrackName>>>,
  pub relay_fetch_requests: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,
  pub relay_subscribe_requests: Arc<RwLock<BTreeMap<u64, SubscribeRequest>>>,
  pub app_config: &'static AppConfig,
  pub relay_next_request_id: Arc<RwLock<u64>>,
}

impl Server {
  pub fn new() -> Self {
    let config = AppConfig::load();

    init_logging(&config.log_folder);

    debug!("Server | App. Config.: {:?}", config);

    Server {
      client_manager: Arc::new(RwLock::new(ClientManager::new())),
      tracks: Arc::new(RwLock::new(HashMap::new())),
      track_aliases: Arc::new(RwLock::new(BTreeMap::new())),
      relay_fetch_requests: Arc::new(RwLock::new(BTreeMap::new())),
      relay_subscribe_requests: Arc::new(RwLock::new(BTreeMap::new())),
      app_config: config,
      relay_next_request_id: Arc::new(RwLock::new(1u64)), // relay's request id starts at 1 and are odd
    }
  }

  pub async fn start(&mut self) -> Result<()> {
    let server_config = self.app_config.build_server_config().await?;
    let server = Endpoint::server(server_config)?;

    info!("MOQtail Relay is running!");
    info!(
      "URL: https://{}:{}",
      self.app_config.host, self.app_config.port
    );

    for id in 0.. {
      let incoming_session = server.accept().await;
      let server = self.clone();
      tokio::spawn(async move {
        match Session::new(incoming_session, server).await {
          Ok(_) => {
            info!("new session: {}", id);
          }
          Err(e) => {
            error!("Error occurred in session {}: {:?}", id, e);
          }
        }
      });
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
