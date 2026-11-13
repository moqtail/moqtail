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
use clap::Parser;
use figment::Figment;
use figment::providers::{Format, Yaml};
use moqtail::model::control::constant::SUPPORTED_VERSIONS;
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use socket2::{Domain, Socket, Type};
use std::net::{Ipv6Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{error, info, warn};
use validator::Validate;
use wtransport::Identity;
use wtransport::quinn::congestion::BbrConfig;
use wtransport::quinn::{self, TransportConfig};

/// Cache expiration strategy
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheExpirationType {
  /// Time-to-live: entries expire after a fixed duration from creation
  Ttl,
  /// Time-to-idle: entries expire after a period of inactivity
  Tti,
}

/// How the relay picks the Track Alias it gives a subscriber. Either is allowed: a
/// publisher may give concurrent subscriptions to one track the same alias or
/// different ones, and subscribers have to handle both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrackAliasMode {
  /// One alias per track: every subscription to it gets the track's relay id.
  PerTrack,
  /// A fresh alias for every subscription, unique within the subscriber's session.
  PerSubscription,
}

/// Upper bound on `io_sockets`. Past the core count extra sockets only add
/// accept loops with nothing to do.
const MAX_IO_SOCKETS: usize = 256;

/// Upper bound on `dedup_retained_groups`. Retention costs groups × objects-per-group,
/// and nothing bounds the second factor, so the first is bounded here.
const MAX_DEDUP_RETAINED_GROUPS: usize = 1000;

/// Everything else is set in the YAML file given to `--config`; see
/// `apps/relay/relay.example.yaml`. Flags given here override the file.
#[derive(Parser, Debug, Default)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
  /// YAML config file
  #[arg(long)]
  pub config: Option<PathBuf>,
  /// Address to bind, e.g. 0.0.0.0:4433 [default: [::]:4433]
  #[arg(long)]
  pub bind: Option<SocketAddr>,
  /// Certificate PEM file [default: apps/relay/cert/cert.pem]
  #[arg(long)]
  pub cert_file: Option<String>,
  /// Private key PEM file [default: apps/relay/cert/key.pem]
  #[arg(long)]
  pub key_file: Option<String>,
}

/// The settings the relay runs on. YAML keys are the field names; a key left out
/// takes the field's `#[default]`, and an unknown key is a startup error.
/// Durations are written with units: `10s`, `500ms`, `30m`.
#[derive(Debug, Clone, Deserialize, Serialize, SmartDefault, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
  /// Address to bind. `[::]` (the default) takes IPv4 and IPv6 alike.
  #[default(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 4433)))]
  pub bind: SocketAddr,
  /// Certificate PEM file. Relative paths here and below resolve against the
  /// directory the relay is started from, not the config file's.
  #[default("apps/relay/cert/cert.pem")]
  pub cert_file: String,
  /// Private key PEM file
  #[default("apps/relay/cert/key.pem")]
  pub key_file: String,
  /// QUIC idle timeout
  #[default(Duration::from_secs(7))]
  #[serde(with = "humantime_serde")]
  pub max_idle_timeout: Duration,
  /// QUIC keep-alive interval
  #[default(Duration::from_secs(3))]
  #[serde(with = "humantime_serde")]
  pub keep_alive_interval: Duration,
  /// Number of cached subgroups/fetches per track
  #[default(1000)]
  pub cache_size: u16,
  /// Folder for the optional object and cache-eviction logs
  #[default("/tmp")]
  pub log_folder: String,
  /// Cache expiration type (ttl or tti)
  #[default(CacheExpirationType::Ttl)]
  pub cache_expiration_type: CacheExpirationType,
  /// Cache expiration duration
  #[default(Duration::from_secs(30 * 60))]
  #[serde(with = "humantime_serde")]
  pub cache_expiration: Duration,
  /// Enable object logging
  pub enable_object_logging: bool,
  /// Enable token logging
  pub enable_token_logging: bool,
  /// Token log file path
  #[default("/tmp/tokens.csv")]
  pub token_log_path: String,
  /// Folder for qlog traces of control messages, one `.sqlog` file per connection.
  /// Unset turns qlog off.
  pub qlog_folder: Option<String>,
  /// Number of UDP sockets to listen on, all bound to the same port with
  /// SO_REUSEPORT. Connections are spread across them by the kernel, and each socket
  /// carries its own I/O readiness state, so drivers stop contending over one.
  /// Linux only: elsewhere the kernel does not distribute across sockets, so this
  /// stays at 1 there.
  #[default(1)]
  pub io_sockets: usize,
  /// UDP send and receive buffer per socket, in KiB (0: kernel default). A large
  /// buffer absorbs fan-out bursts; the kernel caps it at net.core.wmem_max /
  /// rmem_max. Use 0 if the interface queue is a short FIFO.
  #[default(8192)]
  pub socket_buffer_kib: usize,
  /// Maximum number of concurrent request streams a peer may open (applied as the
  /// QUIC max_concurrent_bidi_streams limit).
  #[default(10_000)]
  pub max_request_streams: u64,
  /// Application-level load-shedding limit: once this many request streams are
  /// being served across the relay, new ones are reset with EXCESSIVE_LOAD.
  /// 0 disables load shedding.
  pub max_active_requests: u64,
  /// Max objects queued for a subscriber before its data streams are reset with
  /// TOO_FAR_BEHIND. 0 disables the check.
  pub max_subscriber_lag: u64,
  /// How long an Object may wait in a subscriber's queue before the rest of its group
  /// is skipped for that subscriber. Also caps a subscriber's own Object Delivery
  /// Timeout, which may only be stricter. 0s sets no relay limit.
  #[serde(with = "humantime_serde")]
  pub delivery_timeout: Duration,
  /// Max payload bytes queued for one subscriber. Past it, the rest of the group is
  /// skipped for that subscriber, so a stalled one cannot grow memory without bound.
  /// 0 = unlimited.
  #[default(16 * 1024 * 1024)]
  pub max_subscriber_queue_bytes: usize,
  /// Max PUBLISH streams the relay initiates for one SUBSCRIBE_TRACKS before it
  /// runs out of streams and emits PUBLISH_SKIPPED. 0 = unlimited.
  pub max_publish_streams: u64,
  /// Track Alias given to subscribers: one per subscription, or one per track.
  #[default(TrackAliasMode::PerSubscription)]
  pub track_alias_mode: TrackAliasMode,
  /// Simulate a bandwidth cap per subscriber connection (kbps). 0 = unlimited.
  /// Useful for testing QUIC stream priority scheduling without OS-level throttling.
  pub write_kbps_limit: u64,
  /// URI advertised to clients in GOAWAY so they can migrate to another relay.
  /// Unset reuses the current URI. The runtime value lives on `Server`.
  pub redirect_uri: Option<String>,
  /// Maximum number of upstream fetch gaps before skipping
  #[default(10)]
  pub max_upstream_fetch_gaps: u64,
  /// Timeout for upstream fetch responses
  #[default(Duration::from_secs(10))]
  #[serde(with = "humantime_serde")]
  pub upstream_fetch_timeout: Duration,
  /// How long a publisher has to answer the relay's SUBSCRIBE before it is treated as
  /// declining. Without it a publisher that stays connected and never answers leaves
  /// the subscriber waiting for a response that cannot arrive.
  #[default(Duration::from_secs(10))]
  #[serde(with = "humantime_serde")]
  pub upstream_subscribe_timeout: Duration,
  /// How long a data stream waits for the control message that establishes its track
  /// alias before the stream is abandoned. Giving up costs the whole subgroup, so the
  /// wait is generous; the subscriber side of the same race waits as long.
  #[default(Duration::from_millis(2000))]
  #[serde(with = "humantime_serde")]
  pub track_alias_resolution_timeout: Duration,
  /// How long forwarding waits for the control message that carries a subscriber's
  /// track alias (SUBSCRIBE_OK, or PUBLISH when the relay pushes) to be sent. Objects
  /// queue meanwhile; once it elapses they are forwarded regardless.
  #[default(Duration::from_millis(3000))]
  #[serde(with = "humantime_serde")]
  pub downstream_alias_timeout: Duration,
  /// How long a PUBLISH_DONE waits for the data streams it accounts for to finish
  /// arriving before the track is torn down anyway. The message rides the request
  /// stream and overtakes them, so forwarding it straight through would cut objects
  /// still in flight.
  #[default(Duration::from_millis(2000))]
  #[serde(with = "humantime_serde")]
  pub publish_done_stream_timeout: Duration,
  /// A PUBLISH_DONE for a track that has been receiving datagrams is held until none
  /// has arrived for this long, within `publish_done_stream_timeout`. Datagrams are
  /// not counted in its Stream Count, and the message can overtake the last of them.
  #[default(Duration::from_millis(100))]
  #[serde(with = "humantime_serde")]
  pub publish_done_datagram_quiet: Duration,
  /// How many recent groups of Object ids are remembered per track, to drop duplicates
  /// when several publishers serve the same Track. 0 turns duplicate detection off.
  /// Capped, because the memory this costs also scales with the size of a group; a
  /// publisher more than this many groups behind can slip a duplicate through.
  #[default(30)]
  #[validate(range(max = MAX_DEDUP_RETAINED_GROUPS))]
  pub dedup_retained_groups: usize,
  /// How many Range Filter ranges a subscriber may ask for, in total, for one
  /// subscription or fetch, advertised as MAX_FILTER_RANGES. The filters inside
  /// FILL_PARAMETERS count against a limit of their own. 0 allows no Range Filters.
  #[default(32)]
  pub max_filter_ranges: u64,
  /// Longest a SUBSCRIBE waits for a publisher, whatever the subscriber asks.
  /// Bounds how long held requests tie up streams. 0s never waits.
  #[default(Duration::from_millis(10000))]
  #[serde(with = "humantime_serde")]
  pub max_rendezvous_timeout: Duration,
  /// Enable sender-side track switching (SSTS). Off by default: the setup option
  /// and message parameter it relies on are provisional on an unadopted draft, so
  /// relays that never turn the feature on never put them on the wire.
  pub ssts_enable: bool,
  /// Which SSTS algorithm ids to run and advertise in SETUP (YAML ints, decimal or
  /// 0x hex). Defaults to the default allocation (0) alone: SSTS is a new mechanism,
  /// and any id beyond that first one is an experimental algorithm of this
  /// implementation's, so the relay does not volunteer it. An id the relay does not
  /// implement is a startup error rather than a warning, because advertising an
  /// algorithm this relay cannot run only produces subscriptions that are rejected
  /// one by one. Empty after loading unless `ssts_enable` is set, which is also
  /// what clients negotiate to.
  #[default(vec![0])]
  #[validate(custom(function = "super::ssts::check_algorithm_ids"))]
  pub ssts_algorithms: Vec<u64>,
  /// How long an SSTS client's subgroup stream waits for a graceful finish
  /// before it is reset and its stream slot is freed again. Only applies to
  /// connections that negotiated SSTS; 0s always closes gracefully.
  #[default(Duration::from_millis(1600))]
  #[serde(with = "humantime_serde")]
  pub ssts_discard_timeout: Duration,
  /// This relay's Hop ID in a relay mesh, declared in SETUP. Setting it turns
  /// clustering on; 0 joins the mesh while withholding an identity.
  pub hop_id: Option<u64>,
  /// What this relay charges for its own egress, declared as RELAY_COST. Unset
  /// leaves it out, which peers read as 1, so an unpriced mesh ranks by hop count.
  pub relay_cost: Option<u64>,
  /// Relays to dial and keep a cluster session with (`moqt://host:port`, raw QUIC).
  /// One link carries advertisements both ways; listing each other is fine too.
  pub peers: Vec<String>,
  /// Skip certificate validation when dialing `peers` (self-signed dev certs).
  pub peer_no_cert_validation: bool,
  /// Starts the HTTP API under `/api/v1`.
  pub api_enable: bool,
  /// Address of the API. Unset means `127.0.0.1:4480`.
  pub api_bind: Option<SocketAddr>,
  /// Parts of the API that run: health, state, metrics, latency, qlog, events,
  /// actions. Unset means health, state, metrics, latency and qlog.
  pub api_features: Option<Vec<String>>,
  /// Client address ranges allowed on a network `api_bind`. Unset means loopback
  /// only; ignored on a loopback `api_bind`.
  pub api_allow: Option<Vec<String>>,
  /// When set, API reads need it as a bearer token, except health and readiness.
  #[serde(skip_serializing)]
  pub api_read_token: Option<String>,
  /// Needed for the API's write endpoints; also allows reads.
  #[serde(skip_serializing)]
  pub api_write_token: Option<String>,
  /// Browser origins allowed to call the API, such as the dashboard's.
  pub api_cors_origins: Vec<String>,
  /// A second listener serving only `/api/v1/metrics`, for example to open the
  /// metrics to a monitoring network while the rest of the API stays on loopback.
  pub api_metrics_bind: Option<SocketAddr>,
  /// A token that reads `/api/v1/metrics` and nothing else, for Prometheus.
  #[serde(skip_serializing)]
  pub api_metrics_token: Option<String>,
  /// Closed connections the API keeps a summary of, oldest dropped first.
  #[default(10_000)]
  pub closed_connection_history: usize,
  /// How finely the API keeps object processing time: relay, track or subscription.
  #[default("subscription")]
  pub latency_detail: String,
  /// Measure the processing time of every Nth object only; 1 measures all of them.
  #[default(1)]
  pub latency_sample: u64,
  /// Deliveries slower than this, from arrival to written, go to the API's slowlog.
  #[default(Duration::from_millis(20))]
  #[serde(with = "humantime_serde")]
  pub slowlog_threshold: Duration,
  /// Subscriptions waiting longer than this for their first object go to the slowlog.
  #[default(Duration::from_secs(1))]
  #[serde(with = "humantime_serde")]
  pub slowlog_first_object_threshold: Duration,
  /// Slowlog entries kept.
  #[default(1000)]
  pub slowlog_size: usize,
  /// Namespace prefixes (`a/b`) whose traffic and requests the API counts
  /// separately; a namespace belongs to the longest prefix it falls under.
  pub accounting_zones: Vec<String>,
  /// Publish the relay's own metrics as MoQ tracks under
  /// `metrics.moq.arpa/v1/<metrics_resource_id>`. Needs `api_enable`.
  pub metrics_track_enable: bool,
  /// 16 lowercase hexadecimal characters naming this relay in the metrics
  /// namespace; unset makes one up each time the relay starts.
  pub metrics_resource_id: Option<String>,
}

impl AppConfig {
  pub fn load() -> &'static Self {
    static INSTANCE: OnceLock<AppConfig> = OnceLock::new();
    INSTANCE.get_or_init(|| {
      Self::from_cli(Cli::parse()).unwrap_or_else(|e| {
        eprintln!("error: {e:#}");
        std::process::exit(2);
      })
    })
  }

  /// The config file, with the command line on top, resolved into the settings
  /// the relay runs on.
  pub fn from_cli(cli: Cli) -> Result<Self> {
    let mut figment = Figment::new();
    if let Some(path) = &cli.config {
      figment = figment.merge(Yaml::file_exact(path));
    }
    let mut config: AppConfig = figment.extract()?;
    if let Some(bind) = cli.bind {
      config.bind = bind;
    }
    if let Some(cert_file) = cli.cert_file {
      config.cert_file = cert_file;
    }
    if let Some(key_file) = cli.key_file {
      config.key_file = key_file;
    }
    config.validate()?;
    config.check_cluster()?;
    Ok(config.finalize())
  }

  /// Peers dialed without a Hop ID would come up as plain sessions, and nothing would
  /// ever be advertised to them.
  fn check_cluster(&self) -> Result<()> {
    if !self.peers.is_empty() && self.hop_id.is_none() {
      anyhow::bail!("`peers` needs `hop_id`: without one the relay joins no mesh");
    }
    Ok(())
  }

  fn finalize(mut self) -> Self {
    // When the feature is off the advertised list stays empty, so a client can
    // never negotiate SSTS on this relay however much it wants it.
    // Validation decided which ids are runnable; the feature flag decides
    // whether any of them go on the wire.
    if self.ssts_enable {
      let mut seen = std::collections::HashSet::new();
      self.ssts_algorithms.retain(|id| seen.insert(*id));
    } else {
      self.ssts_algorithms.clear();
    }
    self
  }

  /// Builds the relay's QUIC listeners. WebTransport and raw-QUIC clients share the
  /// same port: the TLS config offers ALPN for both `h3` (WebTransport, added by
  /// `build_default_tls_config`) and the MOQT versions (raw QUIC), and the accept
  /// loops in `Server::start` demultiplex by the negotiated protocol before handing
  /// the connection to either path.
  ///
  /// `io_sockets` decides how many are returned. They share one server config and
  /// differ only in owning separate UDP sockets, which is the point: a socket's I/O
  /// readiness state is behind a single lock, so every connection driver on one
  /// socket contends with every other for it.
  /// The QUIC transport settings every connection uses.
  pub fn transport_config(&self) -> Result<TransportConfig> {
    // set up BBR congestion control
    let mut transport_config = TransportConfig::default();
    transport_config.congestion_controller_factory(Arc::new(BbrConfig::default()));
    transport_config.keep_alive_interval(Some(self.keep_alive_interval));
    transport_config.max_idle_timeout(Some(self.max_idle_timeout.try_into()?));
    // Request flow control: bound the number of concurrent request streams a peer
    // may open.
    transport_config.max_concurrent_bidi_streams(self.max_request_stream_limit());
    Ok(transport_config)
  }

  /// The endpoints, and the server config they share, for connections accepted
  /// with settings of their own.
  pub async fn build_quic_endpoints(&self) -> Result<(Vec<quinn::Endpoint>, quinn::ServerConfig)> {
    let identity = match Identity::load_pemfiles(&self.cert_file, &self.key_file).await {
      Ok(identity) => identity,
      Err(e) => {
        error!("Failed to load identity from PEM files: {:?}", e);
        return Err(e.into());
      }
    };

    let mut tls_config = wtransport::tls::server::build_default_tls_config(identity);

    for version in SUPPORTED_VERSIONS.replace(" ", "").split(",") {
      tls_config.alpn_protocols.push(version.as_bytes().to_vec());
    }
    info!(
      "QUIC ALPN protocols (WebTransport + raw QUIC): {:?}",
      tls_config.alpn_protocols
    );

    let quic_crypto_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)?;

    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto_config));
    server_config.transport_config(Arc::new(self.transport_config()?));

    let count = self.listener_count();
    let runtime = quinn::default_runtime()
      .ok_or_else(|| anyhow::anyhow!("no async runtime found for QUIC endpoint"))?;

    let mut endpoints = Vec::with_capacity(count);
    for _ in 0..count {
      // SO_REUSEPORT only where more than one socket is wanted, so a single-socket
      // relay still fails loudly on a port another process already holds.
      let socket = bind_udp_socket(self.bind, count > 1, self.socket_buffer_kib * 1024)?;
      endpoints.push(quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(server_config.clone()),
        socket,
        runtime.clone(),
      )?);
    }

    Ok((endpoints, server_config))
  }

  /// How many UDP sockets to bind. Only Linux spreads connections across sockets
  /// sharing a port, so asking for more anywhere else would put every connection on
  /// whichever socket the kernel picked and hide the fact behind a config value.
  fn listener_count(&self) -> usize {
    let requested = self.io_sockets.clamp(1, MAX_IO_SOCKETS);
    if requested > 1 && !cfg!(target_os = "linux") {
      warn!(
        "io_sockets {} ignored: only Linux distributes connections across sockets sharing a port",
        requested
      );
      return 1;
    }
    requested
  }

  /// The concurrent request-stream limit applied to the QUIC endpoint via
  /// `max_concurrent_bidi_streams`. Values above the QUIC VarInt maximum are
  /// clamped to it.
  pub fn max_request_stream_limit(&self) -> quinn::VarInt {
    quinn::VarInt::from_u64(self.max_request_streams).unwrap_or(quinn::VarInt::MAX)
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

/// Binds a UDP socket on `addr`. An IPv6 address gets dual-stack explicitly enabled,
/// matching `wtransport`'s own `with_bind_default()` (`IpBindConfig::InAddrAnyDual`)
/// semantics: `quinn::Endpoint::server`'s plain `UdpSocket::bind` leaves dual-stack to
/// the OS default, which isn't guaranteed across platforms.
///
/// `reuse_port` lets several sockets share the port, which is how the relay listens on
/// more than one; the kernel then hashes each connection onto one of them.
fn bind_udp_socket(addr: SocketAddr, reuse_port: bool, buffer_bytes: usize) -> Result<UdpSocket> {
  let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, None)?;
  if addr.is_ipv6() {
    socket.set_only_v6(false)?;
  }
  if reuse_port {
    socket.set_reuse_port(true)?;
  }
  socket.bind(&addr.into())?;
  socket.set_nonblocking(true)?;
  if buffer_bytes > 0 {
    request_buffer_sizes(&socket, buffer_bytes);
  }
  Ok(socket.into())
}

/// Asks for `bytes` each way; warns if `net.core.wmem_max` / `rmem_max` cap it.
fn request_buffer_sizes(socket: &Socket, bytes: usize) {
  let _ = socket.set_send_buffer_size(bytes);
  let _ = socket.set_recv_buffer_size(bytes);
  // Linux stores and reports double the granted size; halve it to compare.
  let reported = |size: std::io::Result<usize>| {
    let size = size.unwrap_or(0);
    if cfg!(target_os = "linux") {
      size / 2
    } else {
      size
    }
  };
  let (send, recv) = (
    reported(socket.send_buffer_size()),
    reported(socket.recv_buffer_size()),
  );
  if send < bytes || recv < bytes {
    warn!(
      "UDP socket buffers are {} KiB send / {} KiB receive, below the {} KiB asked for; raise net.core.wmem_max and net.core.rmem_max to allow more",
      send / 1024,
      recv / 1024,
      bytes / 1024
    );
  } else {
    info!(
      "UDP socket buffers: {} KiB send / {} KiB receive",
      send / 1024,
      recv / 1024
    );
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Loads `yaml` the way `from_cli` does: extract, validate, finalize.
  fn from_yaml(yaml: &str) -> Result<AppConfig> {
    let config: AppConfig = Figment::from(Yaml::string(yaml)).extract()?;
    config.validate()?;
    Ok(config.finalize())
  }

  #[test]
  fn track_alias_mode_defaults_to_per_subscription() {
    assert_eq!(
      AppConfig::default().track_alias_mode,
      TrackAliasMode::PerSubscription
    );
    let config = from_yaml("track_alias_mode: per-track").unwrap();
    assert_eq!(config.track_alias_mode, TrackAliasMode::PerTrack);
  }

  fn test_config(max_request_streams: u64) -> AppConfig {
    AppConfig {
      max_request_streams,
      ..Default::default()
    }
  }

  #[test]
  fn a_single_listener_is_the_default() {
    assert_eq!(AppConfig::default().io_sockets, 1);
    assert_eq!(test_config(10).listener_count(), 1);
  }

  /// Sharing a port only spreads connections on Linux, so asking for more sockets
  /// anywhere else is refused rather than silently piling every connection onto one.
  #[test]
  fn extra_listeners_are_linux_only() {
    let mut config = test_config(10);
    config.io_sockets = 8;
    let expected = if cfg!(target_os = "linux") { 8 } else { 1 };
    assert_eq!(config.listener_count(), expected);
  }

  /// A count of 0 would leave the relay listening on nothing at all.
  #[test]
  fn zero_listeners_is_treated_as_one() {
    let mut config = test_config(10);
    config.io_sockets = 0;
    assert_eq!(config.listener_count(), 1);
  }

  #[test]
  fn peers_need_a_hop_id() {
    let peers = vec!["moqt://relay-b:4433".to_string()];
    let config = AppConfig {
      peers: peers.clone(),
      ..Default::default()
    };
    assert!(config.check_cluster().is_err());
    let config = AppConfig {
      peers,
      hop_id: Some(1),
      ..Default::default()
    };
    assert!(config.check_cluster().is_ok());
  }

  #[test]
  fn test_default_max_request_streams() {
    // The default is the concurrent request-stream limit.
    assert_eq!(AppConfig::default().max_request_streams, 10_000);
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

  fn ssts_algorithms_for(yaml: &str) -> Vec<u64> {
    from_yaml(yaml).unwrap().ssts_algorithms
  }

  #[test]
  fn ssts_advertises_nothing_unless_the_feature_is_on() {
    assert_eq!(ssts_algorithms_for("{}"), Vec::<u64>::new());
    // Asking for algorithms without ssts_enable stays silent: the relay must
    // never put a provisional option on the wire unless it was told to.
    assert_eq!(
      ssts_algorithms_for("ssts_algorithms: [0]"),
      Vec::<u64>::new()
    );
  }

  #[test]
  fn enabling_ssts_runs_the_default_algorithm_alone() {
    assert_eq!(ssts_algorithms_for("ssts_enable: true"), vec![0]);
  }

  #[test]
  fn private_algorithm_ids_are_accepted_in_hex() {
    let yaml = format!(
      "ssts_enable: true\nssts_algorithms: [0, {:#x}]",
      moqtail_ssts::registry::PRIVATE_ALGORITHM_ID_BASE
    );
    assert_eq!(
      ssts_algorithms_for(&yaml),
      vec![0, moqtail_ssts::registry::PRIVATE_ALGORITHM_ID_BASE]
    );
  }

  #[test]
  fn an_unknown_algorithm_id_is_rejected_before_startup() {
    // An algorithm this relay cannot run must not be advertised: a publisher
    // would build switching sets on it and have every subscription rejected.
    assert!(from_yaml("ssts_enable: true\nssts_algorithms: [7]").is_err());
    assert!(from_yaml("ssts_enable: true\nssts_algorithms: [banana]").is_err());
  }

  #[test]
  fn repeated_ssts_ids_are_advertised_once_in_order() {
    let id = moqtail_ssts::registry::PRIVATE_ALGORITHM_ID_BASE;
    let yaml = format!("ssts_enable: true\nssts_algorithms: [0, {id}, 0, {id}]");
    assert_eq!(ssts_algorithms_for(&yaml), vec![0, id]);
  }

  #[test]
  fn the_socket_binds_the_configured_address() {
    let socket = bind_udp_socket("127.0.0.1:0".parse().unwrap(), false, 0).unwrap();
    assert!(socket.local_addr().unwrap().ip().is_loopback());
  }

  #[test]
  fn an_unknown_key_is_rejected() {
    assert!(from_yaml("cache_sise: 10").is_err());
  }

  #[test]
  fn dedup_retained_groups_is_capped() {
    assert!(from_yaml("dedup_retained_groups: 1000").is_ok());
    assert!(from_yaml("dedup_retained_groups: 1001").is_err());
  }

  #[test]
  fn durations_take_units() {
    let config = from_yaml("delivery_timeout: 500ms\ncache_expiration: 30m").unwrap();
    assert_eq!(config.delivery_timeout, Duration::from_millis(500));
    assert_eq!(config.cache_expiration, Duration::from_secs(30 * 60));
    assert!(from_yaml("delivery_timeout: 500").is_err());
  }

  #[test]
  fn the_command_line_overrides_the_file() {
    let dir = std::env::temp_dir().join(format!("relay-config-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("relay.yaml");
    std::fs::write(&path, "bind: 127.0.0.1:5000\ncache_size: 5\n").unwrap();

    let config = AppConfig::from_cli(Cli {
      config: Some(path.clone()),
      ..Default::default()
    })
    .unwrap();
    assert_eq!(config.bind, "127.0.0.1:5000".parse().unwrap());
    assert_eq!(config.cache_size, 5);

    let config = AppConfig::from_cli(Cli::parse_from([
      "relay",
      "--config",
      path.to_str().unwrap(),
      "--bind",
      "0.0.0.0:4433",
    ]))
    .unwrap();
    assert_eq!(config.bind, "0.0.0.0:4433".parse().unwrap());
    assert_eq!(config.cache_size, 5);

    std::fs::remove_dir_all(&dir).unwrap();
  }

  #[test]
  fn a_missing_config_file_is_an_error() {
    let cli = Cli::parse_from(["relay", "--config", "/nonexistent/relay.yaml"]);
    assert!(AppConfig::from_cli(cli).is_err());
  }

  #[test]
  fn the_example_config_loads() {
    from_yaml(include_str!("../../relay.example.yaml")).unwrap();
  }
}
