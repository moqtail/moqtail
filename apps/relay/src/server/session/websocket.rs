// Copyright 2026 The MOQtail Authors
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

//! The relay's WebSocket listener, for clients that can't reach it over QUIC: TLS on
//! TCP, the WebSocket upgrade, then a QMux session that runs like any other. Every
//! WebSocket-specific line of the relay is in this file, apart from the
//! `websocket_bind` setting and the spawn in `server.rs`.

use super::Session;
use crate::server::Server;
use crate::server::config::server_tls_config;
use anyhow::{Context, Result};
use moqtail::model::control::constant::SUPPORTED_VERSIONS;
use moqtail::transport::connection::TransportConnection;
use moqtail::transport::qmux::tokio_tungstenite::tungstenite::handshake::server::{
  ErrorResponse, Request, Response,
};
use moqtail::transport::qmux::tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use moqtail::transport::qmux::{
  QmuxConfig, QmuxSession, Role, tokio_tungstenite, websocket_config,
};
use socket2::{Domain, Socket, Type};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};
use wtransport::Identity;

/// The QMux version this relay speaks, as moq-dev puts it in front of the MoQ version.
const QMUX_PREFIX: &str = "qmux-02.";

/// Accepts WebSocket clients on `bind` until `shutdown` fires.
pub(crate) async fn serve(server: Server, bind: SocketAddr, mut shutdown: watch::Receiver<bool>) {
  let (listener, acceptor) = match listen(&server, bind).await {
    Ok(ready) => ready,
    Err(e) => {
      error!("WebSocket listener on {bind} failed to start: {e:#}");
      return;
    }
  };
  info!("WebSocket (QMux) clients: wss://{bind}");
  loop {
    tokio::select! {
      _ = shutdown.changed() => break,
      accepted = listener.accept() => {
        let (tcp, remote) = match accepted {
          Ok(accepted) => accepted,
          Err(e) => {
            warn!("WebSocket accept failed: {e}");
            continue;
          }
        };
        let server = server.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
          if let Err(e) = accept(tcp, remote, acceptor, server).await {
            error!("WebSocket session from {remote} failed: {e:#}");
            relay_hooks::reporter().connection_failed("handshake");
          }
        });
      }
    }
  }
}

async fn listen(server: &Server, bind: SocketAddr) -> Result<(TcpListener, TlsAcceptor)> {
  let config = server.app_config;
  let identity = Identity::load_pemfiles(&config.cert_file, &config.key_file)
    .await
    .context("loading the TLS certificate")?;
  let mut tls = server_tls_config(&identity)?;
  tls.alpn_protocols = vec![b"http/1.1".to_vec()];

  // Like the UDP sockets: `[::]` takes IPv4 and IPv6 alike.
  let socket = Socket::new(Domain::for_address(bind), Type::STREAM, None)?;
  if bind.is_ipv6() {
    socket.set_only_v6(false)?;
  }
  socket.set_reuse_address(true)?;
  socket.set_nonblocking(true)?;
  socket.bind(&bind.into())?;
  socket.listen(1024)?;
  let listener = TcpListener::from_std(socket.into())?;
  Ok((listener, TlsAcceptor::from(Arc::new(tls))))
}

async fn accept(
  tcp: TcpStream,
  remote: SocketAddr,
  acceptor: TlsAcceptor,
  server: Server,
) -> Result<()> {
  tcp.set_nodelay(true)?;
  let tls = acceptor.accept(tcp).await.context("TLS handshake")?;

  let mut chosen = None;
  // tungstenite's callback signature fixes the large error type.
  #[allow(clippy::result_large_err)]
  let pick_version = |request: &Request, mut response: Response| {
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok());
    let offered = header("sec-websocket-protocol").unwrap_or_default();
    info!(
      "New WebSocket session: remote {remote}, path {:?}, authority {:?}, origin {:?}, user agent {:?}, offered {offered:?}",
      request.uri().path(),
      header("host"),
      header("origin"),
      header("user-agent"),
    );
    let Some(subprotocol) = select_subprotocol(offered) else {
      let mut rejection = ErrorResponse::new(Some(format!(
        "no supported MoQ version; this relay speaks {SUPPORTED_VERSIONS}"
      )));
      *rejection.status_mut() = StatusCode::BAD_REQUEST;
      return Err(rejection);
    };
    let value = HeaderValue::from_str(&subprotocol).expect("chosen from a valid header");
    response
      .headers_mut()
      .insert("sec-websocket-protocol", value);
    chosen = Some(subprotocol);
    Ok(response)
  };
  let ws =
    tokio_tungstenite::accept_hdr_async_with_config(tls, pick_version, Some(websocket_config()))
      .await
      .context("WebSocket upgrade")?;
  info!(
    "WebSocket session from {remote} speaks {}",
    chosen.unwrap_or_default()
  );

  let config = QmuxConfig {
    max_streams_bidi: server.app_config.max_request_stream_limit().into_inner(),
    ..QmuxConfig::default()
  };
  let qmux = QmuxSession::new(ws, Role::Server, remote, config).await?;
  Session::start(TransportConnection::WebSocket(qmux), server, None).await?;
  Ok(())
}

/// The first subprotocol offered whose MoQ version the relay speaks, as offered. Both
/// moq-dev's `qmux-02.moqt-22` and a bare `moqt-22` name QMux carrying `moqt-22`.
fn select_subprotocol(offered: &str) -> Option<String> {
  offered
    .split(',')
    .map(str::trim)
    .find(|protocol| {
      let version = protocol.strip_prefix(QMUX_PREFIX).unwrap_or(protocol);
      SUPPORTED_VERSIONS.split(',').any(|v| v.trim() == version)
    })
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn both_subprotocol_forms_are_accepted_in_the_client_order() {
    assert_eq!(
      select_subprotocol("qmux-02.moqt-22, moqt-22").as_deref(),
      Some("qmux-02.moqt-22")
    );
    assert_eq!(select_subprotocol("moqt-22").as_deref(), Some("moqt-22"));
    assert_eq!(
      select_subprotocol("qmux-01.moqt-22, moqt-99, moqt-22").as_deref(),
      Some("moqt-22"),
      "an older QMux or an unknown MoQ version is skipped"
    );
    assert_eq!(select_subprotocol("qmux-02, webtransport"), None);
    assert_eq!(select_subprotocol(""), None);
  }
}
