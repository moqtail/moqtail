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

use client::Client;
use std::env;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
  init_logging();

  let client_mode = env::args()
    .nth(1)
    .unwrap_or_else(|| "subscriber".to_string());

  let endpoint = env::args()
    .nth(2)
    .unwrap_or_else(|| "https://127.0.0.1:4433".to_string());

  let validate_cert = env::args().nth(3).unwrap_or_else(|| "false".to_string()) == "true";

  info!("Starting client...");
  info!("Client mode: {}", client_mode);
  info!("Endpoint: {}", endpoint);
  info!("Validate cert: {}", validate_cert);

  let mut client = Client::new(endpoint, client_mode, validate_cert);
  client.run().await
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
