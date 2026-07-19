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
use moqtail::model::control::constant::SUPPORTED_VERSIONS;
use moqtail::model::control::control_message::ControlMessageTrait;
use moqtail::model::error::TerminationCode;
use moqtail::model::{
  control::setup::{Setup, SetupSender},
  parameter::setup_option::SetupOption,
};
use moqtail::transport::connection::TransportConnection;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};
use wtransport::endpoint::ConnectOptions;
use wtransport::quinn;
use wtransport::quinn::TransportConfig;
use wtransport::quinn::congestion::BbrConfig;
use wtransport::{ClientConfig, Endpoint, tls};

const CLIENT_SUPPORTED_VERSIONS: &str = "moqt-18";

pub struct MoqConnection {
  pub connection: Arc<TransportConnection>,
  pub control_stream: ControlStreamHandler,
}

impl MoqConnection {
  pub async fn establish(server: &str, no_cert_validation: bool) -> Result<Self> {
    let mut setup_options = vec![SetupOption::new_max_request_id(1000).try_into().unwrap()];

    let connection = match server.strip_prefix("moqt://") {
      Some(authority_and_path) => {
        let (authority, path) = split_authority_and_path(authority_and_path);
        // Raw QUIC only there's no HTTP CONNECT to carry the authority/path,
        // so CLIENT_SETUP must carry them instead. Sending these
        // over WebTransport is a protocol violation, so this only happens here.
        setup_options.push(
          SetupOption::new_authority(authority.to_string())
            .try_into()
            .unwrap(),
        );
        setup_options.push(SetupOption::new_path(path).try_into().unwrap());
        Self::connect_quic(authority, no_cert_validation).await?
      }
      None => Self::connect_webtransport(server, no_cert_validation).await?,
    };

    info!("Connected! Connection ID: {}", connection.stable_id());
    let connection = Arc::new(connection);

    // The control plane is a pair of unidirectional streams. Open our send half
    // and write CLIENT_SETUP first so it goes out without waiting on the
    // server's stream, then accept the server's half.
    info!("Opening control stream and sending SETUP...");
    let mut send_stream = connection.open_uni().await?;
    let client_setup = Setup::new(setup_options);
    let setup_bytes = client_setup
      .serialize()
      .map_err(|e| anyhow::anyhow!("Failed to serialize CLIENT_SETUP: {e:?}"))?;
    send_stream
      .write_all(&setup_bytes)
      .await
      .map_err(|e| anyhow::anyhow!("Failed to send CLIENT_SETUP: {e:?}"))?;

    info!("Waiting for server control stream...");
    let recv_stream = connection.accept_uni().await?;
    let mut control_stream = ControlStreamHandler::new(send_stream, recv_stream);

    // Receive the server's SETUP, which must be the first message. If the server
    // does not support the requested version, the connection is terminated with
    // VersionNegotiationFailed rather than a SETUP arriving.
    info!("Waiting for SETUP...");
    match control_stream.read_setup().await {
      Ok(m) => {
        // AUTHORITY and PATH are client-only; a server sending either closes the
        // session (§10.3.1.1, §10.3.1.2).
        if let Err(code) = m.validate_incoming(SetupSender::Server, connection.kind()) {
          error!("Server sent a client-only setup option: {:?}", code);
          anyhow::bail!("Server sent a client-only setup option: {:?}", code);
        }
        info!("SETUP received: {:?}", m);
      }
      Err(TerminationCode::VersionNegotiationFailed) => {
        anyhow::bail!(
          "Version negotiation failed: server does not support versions: {}",
          SUPPORTED_VERSIONS
        );
      }
      Err(TerminationCode::ProtocolViolation) => {
        anyhow::bail!("Server control stream did not begin with SETUP");
      }
      Err(e) => {
        error!("Failed to receive SETUP: {:?}", e);
        anyhow::bail!("Failed to receive SETUP: {:?}", e);
      }
    };

    Ok(MoqConnection {
      connection,
      control_stream,
    })
  }

  /// Connects over WebTransport (HTTP/3). ALPN here is just `h3` (the default from
  /// `build_default_tls_config`) -- MOQT version negotiation happens via the
  /// `wt-available-protocols` header, not ALPN.
  async fn connect_webtransport(
    server: &str,
    no_cert_validation: bool,
  ) -> Result<TransportConnection> {
    let c = ClientConfig::builder().with_bind_default();

    let tls_config = if no_cert_validation {
      tls::client::build_default_tls_config(
        Arc::new(tls::rustls::RootCertStore::empty()),
        Some(Arc::new(tls::client::NoServerVerification::new())),
      )
    } else {
      tls::client::build_default_tls_config(Arc::new(tls::build_native_cert_store()), None)
    };

    info!("alpn protocols: {:?}", tls_config.alpn_protocols);

    let mut transport_config = TransportConfig::default();
    transport_config.congestion_controller_factory(Arc::new(BbrConfig::default()));

    let config = c
      .with_custom_tls_and_transport(tls_config, transport_config)
      .keep_alive_interval(Some(Duration::from_secs(3)))
      .max_idle_timeout(Some(Duration::from_secs(7)))
      .unwrap()
      .build();

    info!("Connecting via WebTransport to {}", server);
    let endpoint = Endpoint::client(config)?;
    // strings should be surrounded by quotes
    let wt_available_protocols: Vec<String> = CLIENT_SUPPORTED_VERSIONS
      .split(",")
      .map(|s| format!("\"{}\"", s))
      .collect();
    let wt_available_protocols_str = wt_available_protocols.join(", ");
    let options = ConnectOptions::builder(server)
      .add_header("wt-available-protocols", wt_available_protocols_str)
      .build();

    let connection = endpoint.connect(options).await?;
    Ok(TransportConnection::WebTransport(connection))
  }

  /// Connects over raw QUIC (`moqt://authority[/path]`). ALPN here is
  /// the MOQT version string itself (no `h3`) -- that's what the relay's ALPN demux
  /// uses to route the connection to the raw-QUIC path instead of WebTransport.
  async fn connect_quic(authority: &str, no_cert_validation: bool) -> Result<TransportConnection> {
    let (host, port) = match authority.rsplit_once(':') {
      Some((host, port)) => (
        host,
        port
          .parse::<u16>()
          .map_err(|_| anyhow::anyhow!("invalid port in moqt:// authority: {}", authority))?,
      ),
      None => (authority, 443u16),
    };

    let mut tls_config = if no_cert_validation {
      tls::client::build_default_tls_config(
        Arc::new(tls::rustls::RootCertStore::empty()),
        Some(Arc::new(tls::client::NoServerVerification::new())),
      )
    } else {
      tls::client::build_default_tls_config(Arc::new(tls::build_native_cert_store()), None)
    };
    tls_config.alpn_protocols = CLIENT_SUPPORTED_VERSIONS
      .replace(" ", "")
      .split(",")
      .map(|version| version.as_bytes().to_vec())
      .collect();

    info!("alpn protocols: {:?}", tls_config.alpn_protocols);

    let quic_crypto_config = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?;

    let mut transport_config = TransportConfig::default();
    transport_config.congestion_controller_factory(Arc::new(BbrConfig::default()));
    transport_config.keep_alive_interval(Some(Duration::from_secs(3)));
    transport_config.max_idle_timeout(Some(Duration::from_secs(7).try_into()?));

    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_crypto_config));
    client_config.transport_config(Arc::new(transport_config));

    let bind_addr: SocketAddr = if host.parse::<std::net::Ipv6Addr>().is_ok() {
      "[::]:0".parse()?
    } else {
      "0.0.0.0:0".parse()?
    };
    let endpoint = quinn::Endpoint::client(bind_addr)?;

    info!("Connecting via raw QUIC to {}:{}", host, port);
    let remote_addr = resolve_host_port(host, port).await?;
    let connection = endpoint
      .connect_with(client_config, remote_addr, host)?
      .await?;
    Ok(TransportConnection::Quic(connection))
  }
}

async fn resolve_host_port(host: &str, port: u16) -> Result<SocketAddr> {
  if let Ok(ip) = host.parse::<IpAddr>() {
    return Ok(SocketAddr::new(ip, port));
  }
  tokio::net::lookup_host((host, port))
    .await?
    .next()
    .ok_or_else(|| anyhow::anyhow!("DNS resolution failed for host: {}", host))
}

/// Splits `moqt://` URI remainder (everything after the scheme) into the authority
/// (`host[:port]`) and the path-abempty (+ `?query`, if present)
/// e.g. `host:9443/moq-relay` -> (`host:9443`, `/moq-relay`), `host:9443` -> (`host:9443`, ``).
fn split_authority_and_path(authority_and_path: &str) -> (&str, String) {
  match authority_and_path.find('/') {
    Some(idx) => (
      &authority_and_path[..idx],
      authority_and_path[idx..].to_string(),
    ),
    None => (authority_and_path, String::new()),
  }
}
