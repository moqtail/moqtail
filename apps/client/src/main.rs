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

mod cli;
mod connection;
mod fetcher;
mod publisher;
mod stats;
mod subscriber;
mod utils;

use clap::Parser;
use cli::{Cli, Command};
use connection::MoqConnection;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
  init_logging();

  let cli = Cli::parse();

  info!(
    "Starting moqtail client: server={}, namespace={}, track={}",
    cli.server, cli.namespace, cli.track_name
  );

  let moq_conn = MoqConnection::establish(&cli.server, cli.no_cert_validation).await?;

  match cli.command {
    Command::Publish => {
      let config = publisher::PublishConfig {
        namespace: cli.namespace,
        track_name: cli.track_name,
        forwarding_preference: cli.forwarding_preference,
        publish_mode: cli.publish_mode,
        group_count: cli.group_count,
        interval: cli.interval,
        objects_per_group: cli.objects_per_group,
        payload_size: cli.payload_size,
        track_alias: cli
          .track_alias
          .unwrap_or_else(|| rand::random::<u64>() & ((1u64 << 62) - 1)),
      };
      publisher::run(moq_conn, config).await
    }
    Command::Subscribe => {
      subscriber::run(
        moq_conn,
        &cli.namespace,
        &cli.track_name,
        cli.forwarding_preference,
        cli.duration,
      )
      .await
    }
    Command::Fetch => {
      fetcher::run(
        moq_conn,
        &cli.namespace,
        &cli.track_name,
        cli.start_group,
        cli.start_object,
        cli.end_group,
        cli.end_object,
      )
      .await
    }
  }
}

fn init_logging() {
  let env_filter = EnvFilter::builder()
    .with_default_directive(LevelFilter::INFO.into())
    .from_env_lossy();

  tracing_subscriber::fmt()
    .with_target(true)
    .with_level(true)
    .with_env_filter(env_filter)
    .init();
}
