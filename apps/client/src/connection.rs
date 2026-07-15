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
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::error::TerminationCode;
use moqtail::model::{
  control::client_setup::ClientSetup, parameter::setup_parameter::SetupParameter,
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
    let mut setup_parameters = vec![SetupParameter::new_max_request_id(1000).try_into().unwrap()];

    let connection = match server.strip_prefix("moqt://") {
      Some(authority_and_path) => {
        let (authority, path) = split_authority_and_path(authority_and_path);
        // Raw QUIC only there's no HTTP CONNECT to carry the authority/path,
        // so CLIENT_SETUP must carry them instead. Sending these
        // over WebTransport is a protocol violation, so this only happens here.
        setup_parameters.push(
          SetupParameter::new_authority(authority.to_string())
            .try_into()
            .unwrap(),
        );
        setup_parameters.push(SetupParameter::new_path(path).try_into().unwrap());
        Self::connect_quic(authority, no_cert_validation).await?
      }
      None => Self::connect_webtransport(server, no_cert_validation).await?,
    };

    info!("Connected! Connection ID: {}", connection.stable_id());
    let connection = Arc::new(connection);

    // Open bidirectional stream for control messages
    info!("Opening control stream...");
    let (send_stream, recv_stream) = connection.open_bi().await?;
    let mut control_stream = ControlStreamHandler::new(send_stream, recv_stream);

    // Send ClientSetup
    info!("Sending ClientSetup...");
    let client_setup = ClientSetup::new(setup_parameters);
    control_stream.send_impl(&client_setup).await?;

    // Receive ServerSetup
    // If the server does not support the requested version, the connection will
    // be terminated with VersionNegotiationFailed rather than receiving a ServerSetup.
    info!("Waiting for ServerSetup...");
    match control_stream.next_message().await {
      Ok(ControlMessage::ServerSetup(m)) => {
        info!("ServerSetup received: {:?}", m);
      }
      Ok(m) => {
        error!("Unexpected message: {:?}", m);
        anyhow::bail!("Expected ServerSetup, got {:?}", m);
      }
      Err(TerminationCode::VersionNegotiationFailed) => {
        anyhow::bail!(
          "Version negotiation failed: server does not support versions: {}",
          SUPPORTED_VERSIONS
        );
      }
      Err(e) => {
        error!("Failed to receive ServerSetup: {:?}", e);
        anyhow::bail!("Failed to receive ServerSetup: {:?}", e);
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
