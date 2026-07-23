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
use moqtail::model::control::constant::SUPPORTED_VERSIONS;
use socket2::{Domain, Socket, Type};
use std::net::{Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{error, info};
use wtransport::Identity;
use wtransport::quinn::congestion::BbrConfig;
use wtransport::quinn::{self, TransportConfig};

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
  /// Maximum number of concurrent request streams a peer may open (applied as the
  /// QUIC max_concurrent_bidi_streams limit).
  #[arg(long, default_value_t = 10_000)]
  pub max_request_streams: u64,

  /// Application-level load-shedding limit: once this many request streams are
  /// being served across the relay, new ones are reset with EXCESSIVE_LOAD.
  /// 0 disables load shedding.
  #[arg(long, default_value_t = 0)]
  pub max_active_requests: u64,

  /// Max objects queued for a subscriber before its data streams are reset with
  /// TOO_FAR_BEHIND. 0 disables the check.
  #[arg(long, default_value_t = 0)]
  pub max_subscriber_lag: u64,

  /// Max PUBLISH streams the relay initiates for one SUBSCRIBE_TRACKS before it
  /// runs out of streams and emits PUBLISH_BLOCKED. 0 = unlimited.
  #[arg(long, default_value_t = 0)]
  pub max_publish_streams: u64,

  /// Simulate a bandwidth cap per subscriber connection (kbps). 0 = unlimited.
  /// Useful for testing QUIC stream priority scheduling without OS-level throttling.
  #[arg(long, default_value_t = 0)]
  pub write_kbps_limit: u64,

  /// URI advertised to clients in GOAWAY so they can migrate to another relay.
  /// Empty reuses the current URI. Future admin API overrides this at runtime.
  #[arg(long)]
  pub redirect_uri: Option<String>,

  /// Maximum number of upstream fetch gaps before skipping
  #[arg(long, default_value_t = 10)]
  pub max_upstream_fetch_gaps: u64,
  /// Timeout in seconds for upstream fetch responses
  #[arg(long, default_value_t = 10)]
  pub upstream_fetch_timeout_secs: u64,
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
  pub max_request_streams: u64,
  pub max_active_requests: u64,
  pub max_subscriber_lag: u64,
  pub max_publish_streams: u64,
  /// 0 = unlimited. Non-zero caps relay writes to this many kbps per subscriber connection.
  pub write_kbps_limit: u64,
  /// Default GOAWAY redirect URI; the runtime value lives on `Server`.
  pub redirect_uri: Option<String>,
  pub max_upstream_fetch_gaps: u64,
  pub upstream_fetch_timeout: Duration,
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
        max_request_streams: cli.max_request_streams,
        max_active_requests: cli.max_active_requests,
        max_subscriber_lag: cli.max_subscriber_lag,
        max_publish_streams: cli.max_publish_streams,
        write_kbps_limit: cli.write_kbps_limit,
        redirect_uri: cli.redirect_uri,
        max_upstream_fetch_gaps: cli.max_upstream_fetch_gaps,
        upstream_fetch_timeout: Duration::from_secs(cli.upstream_fetch_timeout_secs),
      }
    })
  }

  /// Builds the relay's single QUIC listener. WebTransport and raw-QUIC clients share
  /// this one UDP socket/port: the TLS config offers ALPN for both `h3` (WebTransport,
  /// added by `build_default_tls_config`) and the MOQT versions (raw QUIC), and the
  /// accept loop in `Server::start` demultiplexes by the negotiated protocol before
  /// handing the connection to either path. See `Server::start` for the demux.
  pub async fn build_quic_endpoint(&self) -> Result<quinn::Endpoint> {
    let identity = match Identity::load_pemfiles(&self.cert_file, &self.key_file).await {
      Ok(identity) => identity,
      Err(e) => {
        error!("Failed to load identity from PEM files: {:?}", e);
        return Err(e.into());
      }
    };

    let mut tls_config = wtransport::tls::server::build_default_tls_config(identity);

    for version in SUPPORTED_VERSIONS.replace(" ", "").split(",") {
      // ALPN version strings are quoted, matching the WebTransport convention.
      tls_config
        .alpn_protocols
        .push(format!("\"{version}\"").into_bytes());
    }
    info!(
      "QUIC ALPN protocols (WebTransport + raw QUIC): {:?}",
      tls_config.alpn_protocols
    );

    let quic_crypto_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)?;

    // set up BBR congestion control
    let mut transport_config = TransportConfig::default();
    transport_config.congestion_controller_factory(Arc::new(BbrConfig::default()));
    transport_config.keep_alive_interval(Some(Duration::from_secs(self.keep_alive_interval)));
    transport_config.max_idle_timeout(Some(Duration::from_secs(self.max_idle_timeout).try_into()?));
    // Request flow control: bound the number of concurrent request streams a peer
    // may open.
    transport_config.max_concurrent_bidi_streams(self.max_request_stream_limit());

    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto_config));
    server_config.transport_config(Arc::new(transport_config));

    let socket = bind_dual_stack_udp_socket(self.port)?;
    let runtime = quinn::default_runtime()
      .ok_or_else(|| anyhow::anyhow!("no async runtime found for QUIC endpoint"))?;
    let endpoint = quinn::Endpoint::new(
      quinn::EndpointConfig::default(),
      Some(server_config),
      socket,
      runtime,
    )?;

    Ok(endpoint)
  }

  /// The concurrent request-stream limit applied to the QUIC endpoint via
  /// `max_concurrent_bidi_streams`. Values above the QUIC VarInt maximum are
  /// clamped to it.
  pub fn max_request_stream_limit(&self) -> quinn::VarInt {
    quinn::VarInt::from_u64(self.max_request_streams).unwrap_or(quinn::VarInt::MAX)
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

/// Binds a UDP socket on `[::]:port` with IPv6 dual-stack explicitly enabled, matching
/// `wtransport`'s own `with_bind_default()` (`IpBindConfig::InAddrAnyDual`) semantics.
/// `quinn::Endpoint::server`'s plain `UdpSocket::bind` leaves dual-stack to the OS
/// default, which isn't guaranteed across platforms, so this is done explicitly.
fn bind_dual_stack_udp_socket(port: u16) -> Result<UdpSocket> {
  let socket = Socket::new(Domain::IPV6, Type::DGRAM, None)?;
  socket.set_only_v6(false)?;
  let addr: SocketAddr = (Ipv6Addr::UNSPECIFIED, port).into();
  socket.bind(&addr.into())?;
  socket.set_nonblocking(true)?;
  Ok(socket.into())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn test_config(max_request_streams: u64) -> AppConfig {
    AppConfig {
      port: 4433,
      host: "localhost".to_string(),
      cert_file: "apps/relay/cert/cert.pem".to_string(),
      key_file: "apps/relay/cert/key.pem".to_string(),
      max_idle_timeout: 7,
      keep_alive_interval: 3,
      cache_size: 1000,
      log_folder: "/tmp".to_string(),
      cache_expiration_type: CacheExpirationType::Ttl,
      cache_expiration_minutes: 30,
      enable_object_logging: false,
      enable_token_logging: false,
      token_log_path: "/tmp/moqtail_relay_tokens.csv".to_string(),
      max_request_streams,
      max_active_requests: 0,
      max_subscriber_lag: 0,
      max_publish_streams: 0,
      write_kbps_limit: 0,
      redirect_uri: None,
      max_upstream_fetch_gaps: 10,
      upstream_fetch_timeout: Duration::from_secs(10),
    }
  }

  #[test]
  fn test_default_max_request_streams() {
    // The CLI default is the concurrent request-stream limit.
    let cli = Cli::parse_from(["relay"]);
    assert_eq!(cli.max_request_streams, 10_000);
  }

  #[test]
  fn max_request_stream_limit_is_applied_to_the_endpoint() {
    // The configured limit becomes the QUIC max_concurrent_bidi_streams value.
    let config = test_config(2_500);
    assert_eq!(
      config.max_request_stream_limit(),
      quinn::VarInt::from_u64(2_500).unwrap()
    );
    // A value above the QUIC VarInt maximum is clamped rather than rejected.
    let clamped = test_config(u64::MAX);
    assert_eq!(clamped.max_request_stream_limit(), quinn::VarInt::MAX);
  }
}
