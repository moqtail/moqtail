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

use crate::cli::Transport;
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
  // Held for the session's lifetime so the control stream is not closed (which
  // would be a protocol violation). Requests use their own bidi streams, so it is
  // otherwise unused after SETUP.
  #[allow(dead_code)]
  pub control_stream: ControlStreamHandler,
}

impl MoqConnection {
  pub async fn establish(
    server: &str,
    no_cert_validation: bool,
    transport: Transport,
  ) -> Result<Self> {
    let mut setup_options = Vec::new();

    // moqt:// is the single input scheme; the transport is chosen separately.
    let url = MoqtUrl::parse(server)?;
    if let Some(fragment) = &url.fragment {
      // The fragment is processed locally and never sent to the server.
      info!(
        "moqt:// fragment (local-only): {}:{}",
        fragment.kind, fragment.value
      );
    }

    let connection = match transport {
      Transport::Quic => {
        // Native QUIC has no HTTP CONNECT to carry the authority/path, so
        // CLIENT_SETUP carries them instead. Sending AUTHORITY over WebTransport
        // is a protocol violation, so this only happens on the QUIC path.
        setup_options.push(
          SetupOption::new_authority(url.authority.clone())
            .try_into()
            .unwrap(),
        );
        setup_options.push(
          SetupOption::new_path(url.path_and_query())
            .try_into()
            .unwrap(),
        );
        Self::connect_quic(&url.authority, no_cert_validation).await?
      }
      Transport::WebTransport => {
        // WebTransport derives an https:// URI from the moqt:// URL; the CONNECT
        // request carries the authority/path, so no AUTHORITY/PATH setup options.
        Self::connect_webtransport(&url.to_https(), no_cert_validation).await?
      }
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
        // session.
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
      // ALPN version strings are quoted, matching the WebTransport convention.
      .map(|version| format!("\"{version}\"").into_bytes())
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

/// A local-only fragment, `#<type>:<value>`. Not transmitted to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MoqtFragment {
  kind: String,
  value: String,
}

/// A parsed `moqt://authority/path[?query][#fragment]` URI.
///
/// `moqt-URI = "moqt" "://" authority path-abempty [ "?" query ]`, with an
/// optional local-only fragment. `authority` is `host[:port]`; `path` keeps its
/// leading `/` (empty when absent); `query` excludes the `?`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MoqtUrl {
  authority: String,
  path: String,
  query: Option<String>,
  fragment: Option<MoqtFragment>,
}

impl MoqtUrl {
  fn parse(input: &str) -> Result<Self> {
    let rest = input
      .strip_prefix("moqt://")
      .ok_or_else(|| anyhow::anyhow!("server URL must use the moqt:// scheme: {input}"))?;

    // The fragment is split off first; it never reaches authority/path/query.
    let (rest, fragment) = match rest.split_once('#') {
      Some((before, frag)) => (before, Some(parse_fragment(frag)?)),
      None => (rest, None),
    };

    // Then the query, then the path, leaving the authority.
    let (rest, query) = match rest.split_once('?') {
      Some((before, q)) => (before, Some(q.to_string())),
      None => (rest, None),
    };

    let (authority, path) = match rest.find('/') {
      Some(idx) => (&rest[..idx], rest[idx..].to_string()),
      None => (rest, String::new()),
    };

    if authority.is_empty() {
      anyhow::bail!("moqt:// URL has an empty authority: {input}");
    }

    Ok(Self {
      authority: authority.to_string(),
      path,
      query,
      fragment,
    })
  }

  /// The `path[?query]` string carried in the PATH setup option on native QUIC.
  fn path_and_query(&self) -> String {
    match &self.query {
      Some(q) => format!("{}?{}", self.path, q),
      None => self.path.clone(),
    }
  }

  /// The equivalent `https://` URL for WebTransport, dropping the local fragment.
  fn to_https(&self) -> String {
    format!("https://{}{}", self.authority, self.path_and_query())
  }
}

/// Parses a fragment `<type>:<value>`. The type identifier must be ASCII
/// lowercase letters, digits, and hyphens; the value is opaque here.
fn parse_fragment(fragment: &str) -> Result<MoqtFragment> {
  let (kind, value) = fragment
    .split_once(':')
    .ok_or_else(|| anyhow::anyhow!("moqt:// fragment must be <type>:<value>, got #{fragment}"))?;
  if kind.is_empty()
    || !kind
      .bytes()
      .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
  {
    anyhow::bail!("moqt:// fragment type must be [a-z0-9-]+, got #{fragment}");
  }
  Ok(MoqtFragment {
    kind: kind.to_string(),
    value: value.to_string(),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_authority_only() {
    let url = MoqtUrl::parse("moqt://host:4433").unwrap();
    assert_eq!(url.authority, "host:4433");
    assert_eq!(url.path, "");
    assert_eq!(url.query, None);
    assert_eq!(url.fragment, None);
    assert_eq!(url.to_https(), "https://host:4433");
    assert_eq!(url.path_and_query(), "");
  }

  #[test]
  fn parses_path_and_query() {
    let url = MoqtUrl::parse("moqt://host:4433/moq-relay?a=1").unwrap();
    assert_eq!(url.authority, "host:4433");
    assert_eq!(url.path, "/moq-relay");
    assert_eq!(url.query.as_deref(), Some("a=1"));
    assert_eq!(url.path_and_query(), "/moq-relay?a=1");
    assert_eq!(url.to_https(), "https://host:4433/moq-relay?a=1");
  }

  #[test]
  fn fragment_is_parsed_and_stripped_from_https_and_path() {
    let url = MoqtUrl::parse("moqt://host/app?q=1#warp:abc").unwrap();
    assert_eq!(
      url.fragment,
      Some(MoqtFragment {
        kind: "warp".to_string(),
        value: "abc".to_string(),
      })
    );
    // The fragment never appears in the transmitted URL or path.
    assert_eq!(url.to_https(), "https://host/app?q=1");
    assert_eq!(url.path_and_query(), "/app?q=1");
  }

  #[test]
  fn fragment_value_may_contain_colons() {
    let url = MoqtUrl::parse("moqt://host#loc:a:b:c").unwrap();
    let f = url.fragment.unwrap();
    assert_eq!(f.kind, "loc");
    assert_eq!(f.value, "a:b:c");
  }

  #[test]
  fn rejects_non_moqt_scheme() {
    assert!(MoqtUrl::parse("https://host:4433").is_err());
  }

  #[test]
  fn rejects_empty_authority() {
    assert!(MoqtUrl::parse("moqt:///path").is_err());
  }

  #[test]
  fn rejects_malformed_fragments() {
    // No colon.
    assert!(MoqtUrl::parse("moqt://host#nocolon").is_err());
    // Empty type.
    assert!(MoqtUrl::parse("moqt://host#:value").is_err());
    // Invalid type characters (uppercase).
    assert!(MoqtUrl::parse("moqt://host#Warp:abc").is_err());
  }
}
