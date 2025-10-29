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

use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::sync::OnceLock;
use std::time::Duration;
use tracing::error;
use wtransport::{Identity, ServerConfig};

/// Cache expiration strategy
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CacheExpirationType {
  /// Time-to-live: entries expire after a fixed duration from creation
  Ttl,
  /// Time-to-idle: entries expire after a period of inactivity
  Tti,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
  /// Port to bind
  #[arg(long, default_value_t = 4433)]
  pub port: u16,
  /// Host to bind
  #[arg(long, default_value = "localhost")]
  pub host: String,
  /// Certificate PEM file
  #[arg(long, default_value = "apps/relay/cert/cert.pem")]
  pub cert_file: String,
  /// Private key PEM file
  #[arg(long, default_value = "apps/relay/cert/key.pem")]
  pub key_file: String,
  /// Number of cached subgroups/fetches per track
  #[arg(long, default_value_t = 1000)]
  pub cache_size: u16,
  /// Cache grow ratio before evicting - allows cache to grow to this multiple of cache_size before evicting
  #[arg(long, default_value_t = 7)]
  pub max_idle_timeout: u64,
  #[arg(long, default_value_t = 3)]
  pub keep_alive_interval: u64,
  #[arg(long, default_value = "/tmp")]
  pub log_folder: String,
  /// Cache expiration type (ttl or tti)
  #[arg(long, value_enum, default_value = "ttl")]
  pub cache_expiration_type: CacheExpirationType,
  /// Cache expiration duration in minutes
  #[arg(long, default_value_t = 30)]
  pub cache_expiration_minutes: u64,
  /// Enable object logging
  #[arg(long, default_value_t = false)]
  pub enable_object_logging: bool,
  /// Enable token logging
  #[arg(long, default_value_t = false)]
  pub enable_token_logging: bool,
  /// Token log file path
  #[arg(long, default_value = "/tmp/tokens.csv")]
  pub token_log_path: String,
  /// Initial maximum request ID
  #[arg(long, default_value_t = u64::MAX / 8)]
  pub initial_max_request_id: u64,
}
#[derive(Debug, Clone)]
pub struct AppConfig {
  pub port: u16,
  pub host: String,
  pub cert_file: String,
  pub key_file: String,
  pub max_idle_timeout: u64,
  pub keep_alive_interval: u64,
  pub cache_size: u16,
  pub log_folder: String,
  pub cache_expiration_type: CacheExpirationType,
  pub cache_expiration_minutes: u64,
  pub enable_object_logging: bool,
  pub enable_token_logging: bool,
  pub token_log_path: String,
  pub initial_max_request_id: u64,
}

impl AppConfig {
  pub fn load() -> &'static Self {
    static INSTANCE: OnceLock<AppConfig> = OnceLock::new();
    INSTANCE.get_or_init(|| {
      let cli = Cli::parse();
      AppConfig {
        port: cli.port,
        host: cli.host,
        cert_file: cli.cert_file,
        key_file: cli.key_file,
        max_idle_timeout: cli.max_idle_timeout,
        keep_alive_interval: cli.keep_alive_interval,
        cache_size: cli.cache_size,
        log_folder: cli.log_folder,
        cache_expiration_type: cli.cache_expiration_type,
        cache_expiration_minutes: cli.cache_expiration_minutes,
        enable_object_logging: cli.enable_object_logging,
        enable_token_logging: cli.enable_token_logging,
        token_log_path: cli.token_log_path,
        initial_max_request_id: cli.initial_max_request_id,
      }
    })
  }

  pub async fn build_server_config(&self) -> Result<ServerConfig> {
    let identity = match Identity::load_pemfiles(&self.cert_file, &self.key_file).await {
      Ok(identity) => identity,
      Err(e) => {
        error!("Failed to load identity from PEM files: {:?}", e);
        return Err(e.into());
      }
    };

    let config = ServerConfig::builder()
      .with_bind_default(self.port)
      .with_identity(identity)
      .keep_alive_interval(Some(Duration::from_secs(self.keep_alive_interval)))
      .max_idle_timeout(Some(Duration::from_secs(self.max_idle_timeout)))
      .unwrap()
      .build();

    Ok(config)
  }

  /// Get cache expiration duration
  pub fn get_cache_expiration_duration(&self) -> Duration {
    Duration::from_secs(self.cache_expiration_minutes * 60)
  }

  /// Check if cache uses time-to-live expiration
  #[allow(dead_code)]
  pub fn is_cache_ttl(&self) -> bool {
    matches!(self.cache_expiration_type, CacheExpirationType::Ttl)
  }

  /// Check if cache uses time-to-idle expiration
  #[allow(dead_code)]
  pub fn is_cache_tti(&self) -> bool {
    matches!(self.cache_expiration_type, CacheExpirationType::Tti)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_default_initial_max_request_id() {
    // Test that the default value for initial_max_request_id is u64::MAX
    let cli = Cli {
      port: 4433,
      host: "localhost".to_string(),
      cert_file: "apps/relay/cert/cert.pem".to_string(),
      key_file: "apps/relay/cert/key.pem".to_string(),
      cache_size: 1000,
      max_idle_timeout: 7,
      keep_alive_interval: 3,
      log_folder: "/tmp".to_string(),
      cache_expiration_type: CacheExpirationType::Ttl,
      cache_expiration_minutes: 30,
      enable_object_logging: false,
      enable_token_logging: false,
      token_log_path: "/tmp/moqtail_relay_tokens.csv".to_string(),
      initial_max_request_id: u64::MAX / 8,
    };

    let config = AppConfig {
      port: cli.port,
      host: cli.host,
      cert_file: cli.cert_file,
      key_file: cli.key_file,
      max_idle_timeout: cli.max_idle_timeout,
      keep_alive_interval: cli.keep_alive_interval,
      cache_size: cli.cache_size,
      log_folder: cli.log_folder,
      cache_expiration_type: cli.cache_expiration_type,
      cache_expiration_minutes: cli.cache_expiration_minutes,
      enable_object_logging: cli.enable_object_logging,
      enable_token_logging: cli.enable_token_logging,
      token_log_path: cli.token_log_path,
      initial_max_request_id: cli.initial_max_request_id,
    };

    assert_eq!(config.initial_max_request_id, u64::MAX / 8);
  }
}
