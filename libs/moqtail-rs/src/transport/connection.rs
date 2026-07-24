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

//! Transport-agnostic wrappers over the two QUIC-based backends MOQtail speaks:
//! WebTransport (via `wtransport`) and raw QUIC (via `wtransport::quinn`, the exact
//! quinn crate wtransport uses internally). Both backends have near-identical APIs
//! (wtransport's stream types are literal newtypes over quinn's), so these enums just
//! pick the right inner call and normalize the handful of places where the two
//! diverge (error types, and `finish`/`set_priority` sync-vs-async).

use bytes::Bytes;
use std::net::SocketAddr;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use wtransport::quinn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
  WebTransport,
  Quic,
}

impl std::fmt::Display for TransportKind {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      TransportKind::WebTransport => write!(f, "webtransport"),
      TransportKind::Quic => write!(f, "quic"),
    }
  }
}

#[derive(Debug, Error)]
pub enum TransportConnectionError {
  #[error("application closed")]
  ApplicationClosed,
  #[error("{0}")]
  Other(String),
}

impl From<wtransport::error::ConnectionError> for TransportConnectionError {
  fn from(e: wtransport::error::ConnectionError) -> Self {
    match e {
      wtransport::error::ConnectionError::ApplicationClosed(_) => Self::ApplicationClosed,
      other => Self::Other(format!("{other:?}")),
    }
  }
}

impl From<quinn::ConnectionError> for TransportConnectionError {
  fn from(e: quinn::ConnectionError) -> Self {
    match e {
      quinn::ConnectionError::ApplicationClosed(_) => Self::ApplicationClosed,
      other => Self::Other(format!("{other:?}")),
    }
  }
}

impl From<wtransport::error::StreamOpeningError> for TransportConnectionError {
  fn from(e: wtransport::error::StreamOpeningError) -> Self {
    Self::Other(format!("{e:?}"))
  }
}

impl From<wtransport::error::SendDatagramError> for TransportConnectionError {
  fn from(e: wtransport::error::SendDatagramError) -> Self {
    Self::Other(format!("{e:?}"))
  }
}

impl From<quinn::SendDatagramError> for TransportConnectionError {
  fn from(e: quinn::SendDatagramError) -> Self {
    Self::Other(format!("{e:?}"))
  }
}

#[derive(Debug, Error)]
pub enum TransportWriteError {
  #[error("stream closed or stopped")]
  ClosedOrStopped,
  #[error("{0}")]
  Other(String),
}

impl From<wtransport::error::StreamWriteError> for TransportWriteError {
  fn from(e: wtransport::error::StreamWriteError) -> Self {
    use wtransport::error::StreamWriteError as E;
    match e {
      E::Closed | E::Stopped(_) => Self::ClosedOrStopped,
      other => Self::Other(format!("{other:?}")),
    }
  }
}

impl From<quinn::WriteError> for TransportWriteError {
  fn from(e: quinn::WriteError) -> Self {
    use quinn::WriteError as E;
    match e {
      E::ClosedStream | E::Stopped(_) => Self::ClosedOrStopped,
      other => Self::Other(format!("{other:?}")),
    }
  }
}

impl From<quinn::ClosedStream> for TransportWriteError {
  fn from(_: quinn::ClosedStream) -> Self {
    Self::ClosedOrStopped
  }
}

#[derive(Debug, Error)]
pub enum TransportReadError {
  #[error("not connected")]
  NotConnected,
  /// The peer reset the stream with this application error code.
  #[error("stream reset with code {0:#x}")]
  Reset(u64),
  #[error("{0}")]
  Other(String),
}

impl From<wtransport::error::StreamReadError> for TransportReadError {
  fn from(e: wtransport::error::StreamReadError) -> Self {
    use wtransport::error::StreamReadError as E;
    match e {
      E::NotConnected => Self::NotConnected,
      E::Reset(code) => Self::Reset(code.into_inner()),
      other => Self::Other(format!("{other:?}")),
    }
  }
}

impl From<quinn::ReadError> for TransportReadError {
  fn from(e: quinn::ReadError) -> Self {
    use quinn::ReadError as E;
    match e {
      E::ConnectionLost(_) | E::ClosedStream => Self::NotConnected,
      E::Reset(code) => Self::Reset(code.into_inner()),
      other => Self::Other(format!("{other:?}")),
    }
  }
}

#[derive(Debug)]
pub enum TransportSendStream {
  WebTransport(wtransport::SendStream),
  Quic(quinn::SendStream),
}

impl TransportSendStream {
  pub async fn write_all(&mut self, buf: &[u8]) -> Result<(), TransportWriteError> {
    match self {
      Self::WebTransport(s) => s.write_all(buf).await.map_err(Into::into),
      Self::Quic(s) => s.write_all(buf).await.map_err(Into::into),
    }
  }

  pub async fn finish(&mut self) -> Result<(), TransportWriteError> {
    match self {
      Self::WebTransport(s) => s.finish().await.map_err(Into::into),
      Self::Quic(s) => s.finish().map_err(Into::into),
    }
  }

  pub async fn flush(&mut self) -> Result<(), TransportWriteError> {
    let result = match self {
      Self::WebTransport(s) => s.flush().await,
      Self::Quic(s) => s.flush().await,
    };
    result.map_err(|e| TransportWriteError::Other(e.to_string()))
  }

  /// Best-effort: if the stream is already closed, setting priority is a no-op.
  pub fn set_priority(&self, priority: i32) {
    match self {
      Self::WebTransport(s) => s.set_priority(priority),
      Self::Quic(s) => {
        let _ = s.set_priority(priority);
      }
    }
  }

  /// Abruptly terminates the send side of the stream with an application error
  /// code (QUIC RESET_STREAM). The peer observes the code via `read` returning
  /// [`TransportReadError::Reset`].
  pub fn reset(&mut self, code: u64) -> Result<(), TransportWriteError> {
    match self {
      Self::WebTransport(s) => {
        let vi = wtransport::VarInt::try_from_u64(code)
          .map_err(|_| TransportWriteError::Other("reset code exceeds VarInt max".to_string()))?;
        s.reset(vi)
          .map_err(|_| TransportWriteError::ClosedOrStopped)
      }
      Self::Quic(s) => {
        let vi = quinn::VarInt::from_u64(code)
          .map_err(|_| TransportWriteError::Other("reset code exceeds VarInt max".to_string()))?;
        s.reset(vi).map_err(Into::into)
      }
    }
  }
}

#[derive(Debug)]
pub enum TransportRecvStream {
  WebTransport(wtransport::RecvStream),
  Quic(quinn::RecvStream),
}

impl TransportRecvStream {
  pub async fn read(&mut self, buf: &mut [u8]) -> Result<Option<usize>, TransportReadError> {
    match self {
      Self::WebTransport(s) => s.read(buf).await.map_err(Into::into),
      Self::Quic(s) => s.read(buf).await.map_err(Into::into),
    }
  }

  /// Asks the peer to stop sending on this stream (QUIC STOP_SENDING) with an
  /// application error code, consuming the stream. Simply dropping the stream
  /// instead sends STOP_SENDING with code 0, which the peer decodes as
  /// InternalError; passing an explicit code keeps a cancellation coherent with
  /// the RESET_STREAM sent on the peer's paired send half.
  pub fn stop(self, code: u64) {
    match self {
      Self::WebTransport(s) => {
        if let Ok(vi) = wtransport::VarInt::try_from_u64(code) {
          s.stop(vi);
        }
      }
      Self::Quic(mut s) => {
        if let Ok(vi) = quinn::VarInt::from_u64(code) {
          let _ = s.stop(vi);
        }
      }
    }
  }
}

#[derive(Debug, Clone)]
pub enum TransportConnection {
  WebTransport(wtransport::Connection),
  Quic(quinn::Connection),
}

impl TransportConnection {
  pub fn kind(&self) -> TransportKind {
    match self {
      Self::WebTransport(_) => TransportKind::WebTransport,
      Self::Quic(_) => TransportKind::Quic,
    }
  }

  pub fn stable_id(&self) -> usize {
    match self {
      Self::WebTransport(c) => c.stable_id(),
      Self::Quic(c) => c.stable_id(),
    }
  }

  pub fn remote_address(&self) -> SocketAddr {
    match self {
      Self::WebTransport(c) => c.remote_address(),
      Self::Quic(c) => c.remote_address(),
    }
  }

  /// Closes the connection with an application error code and reason.
  pub fn close(&self, error_code: u32, reason: &[u8]) {
    match self {
      Self::WebTransport(c) => c.close(error_code.into(), reason),
      Self::Quic(c) => c.close(error_code.into(), reason),
    }
  }

  /// Waits for the connection to be closed, for any reason.
  pub async fn closed(&self) {
    match self {
      Self::WebTransport(c) => {
        c.closed().await;
      }
      Self::Quic(c) => {
        c.closed().await;
      }
    }
  }

  pub async fn open_bi(
    &self,
  ) -> Result<(TransportSendStream, TransportRecvStream), TransportConnectionError> {
    match self {
      Self::WebTransport(c) => {
        let (send, recv) = c.open_bi().await?.await?;
        Ok((
          TransportSendStream::WebTransport(send),
          TransportRecvStream::WebTransport(recv),
        ))
      }
      Self::Quic(c) => {
        let (send, recv) = c.open_bi().await?;
        Ok((
          TransportSendStream::Quic(send),
          TransportRecvStream::Quic(recv),
        ))
      }
    }
  }

  pub async fn accept_bi(
    &self,
  ) -> Result<(TransportSendStream, TransportRecvStream), TransportConnectionError> {
    match self {
      Self::WebTransport(c) => {
        let (send, recv) = c.accept_bi().await?;
        Ok((
          TransportSendStream::WebTransport(send),
          TransportRecvStream::WebTransport(recv),
        ))
      }
      Self::Quic(c) => {
        let (send, recv) = c.accept_bi().await?;
        Ok((
          TransportSendStream::Quic(send),
          TransportRecvStream::Quic(recv),
        ))
      }
    }
  }

  pub async fn open_uni(&self) -> Result<TransportSendStream, TransportConnectionError> {
    match self {
      Self::WebTransport(c) => {
        let send = c.open_uni().await?.await?;
        Ok(TransportSendStream::WebTransport(send))
      }
      Self::Quic(c) => {
        let send = c.open_uni().await?;
        Ok(TransportSendStream::Quic(send))
      }
    }
  }

  pub async fn accept_uni(&self) -> Result<TransportRecvStream, TransportConnectionError> {
    match self {
      Self::WebTransport(c) => Ok(TransportRecvStream::WebTransport(c.accept_uni().await?)),
      Self::Quic(c) => Ok(TransportRecvStream::Quic(c.accept_uni().await?)),
    }
  }

  pub async fn receive_datagram(&self) -> Result<Bytes, TransportConnectionError> {
    match self {
      Self::WebTransport(c) => Ok(Bytes::from(c.receive_datagram().await?.payload().to_vec())),
      Self::Quic(c) => Ok(c.read_datagram().await?),
    }
  }

  pub fn send_datagram(&self, payload: Bytes) -> Result<(), TransportConnectionError> {
    match self {
      Self::WebTransport(c) => c.send_datagram(payload).map_err(Into::into),
      Self::Quic(c) => c.send_datagram(payload).map_err(Into::into),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::error::Error;
  use std::sync::Arc;
  use wtransport::endpoint::IntoConnectOptions;

  const TEST_ALPN: &[u8] = b"test-moqt";

  async fn webtransport_pair() -> Result<(TransportConnection, TransportConnection), Box<dyn Error>>
  {
    let server_identity = wtransport::Identity::self_signed(std::iter::once("localhost"))?;
    let server_cert_hash = server_identity.certificate_chain().as_slice()[0].hash();

    let server_config = wtransport::ServerConfig::builder()
      .with_bind_address("127.0.0.1:0".parse()?)
      .with_identity(server_identity)
      .build();
    let server_endpoint = wtransport::Endpoint::server(server_config)?;
    let server_addr = server_endpoint.local_addr()?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
      let incoming = server_endpoint.accept().await;
      let session_request = incoming.await.expect("session request");
      let connection = session_request.accept().await.expect("accept");
      let _ = tx.send(connection);
    });

    let client_config = wtransport::ClientConfig::builder()
      .with_bind_default()
      .with_server_certificate_hashes(vec![server_cert_hash])
      .build();
    let client_endpoint = wtransport::Endpoint::client(client_config)?;
    let client = client_endpoint
      .connect(format!("https://{}:{}", server_addr.ip(), server_addr.port()).into_options())
      .await?;

    let server = rx.await?;

    Ok((
      TransportConnection::WebTransport(client),
      TransportConnection::WebTransport(server),
    ))
  }

  async fn quic_pair() -> Result<(TransportConnection, TransportConnection), Box<dyn Error>> {
    let server_identity = wtransport::Identity::self_signed(std::iter::once("localhost"))?;
    let server_cert_hash = server_identity.certificate_chain().as_slice()[0].hash();

    let mut server_tls = wtransport::tls::server::build_default_tls_config(server_identity);
    server_tls.alpn_protocols = vec![TEST_ALPN.to_vec()];
    let quic_server_config = quinn::crypto::rustls::QuicServerConfig::try_from(server_tls)?;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server_config));
    let server_endpoint = quinn::Endpoint::server(server_config, "127.0.0.1:0".parse()?)?;
    let server_addr = server_endpoint.local_addr()?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
      let incoming = server_endpoint.accept().await.expect("incoming");
      let connection = incoming.await.expect("connection");
      let _ = tx.send(connection);
    });

    let mut client_tls = wtransport::tls::client::build_default_tls_config(
      Arc::new(wtransport::tls::rustls::RootCertStore::empty()),
      Some(Arc::new(
        wtransport::tls::client::NoServerVerification::new(),
      )),
    );
    client_tls.alpn_protocols = vec![TEST_ALPN.to_vec()];
    let _ = server_cert_hash; // NoServerVerification skips cert checks for this test
    let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(client_tls)?;
    let client_config = quinn::ClientConfig::new(Arc::new(quic_client_config));

    let client_endpoint = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
    let client = client_endpoint
      .connect_with(client_config, server_addr, "localhost")?
      .await?;

    let server = rx.await?;

    Ok((
      TransportConnection::Quic(client),
      TransportConnection::Quic(server),
    ))
  }

  async fn roundtrip_bi(client: TransportConnection, server: TransportConnection) {
    // Raw QUIC sends nothing on the wire for an opened-but-unwritten stream (unlike
    // WebTransport, whose open_bi() writes a stream-type header immediately), so write
    // before waiting on the peer's accept_bi() or it can idle out waiting for a stream
    // it was never told about.
    let (mut client_send, _client_recv) = client.open_bi().await.expect("client open_bi");
    client_send.write_all(b"hello").await.expect("write_all");

    let (_server_send, mut server_recv) = server.accept_bi().await.expect("server accept_bi");

    let mut buf = [0u8; 5];
    let mut read = 0;
    while read < buf.len() {
      let n = server_recv
        .read(&mut buf[read..])
        .await
        .expect("read")
        .expect("stream open");
      read += n;
    }
    assert_eq!(&buf, b"hello");
  }

  #[tokio::test]
  async fn webtransport_backend_roundtrip() {
    let (client, server) = webtransport_pair().await.expect("webtransport_pair");
    roundtrip_bi(client, server).await;
  }

  #[tokio::test]
  async fn quic_backend_roundtrip() {
    let (client, server) = quic_pair().await.expect("quic_pair");
    roundtrip_bi(client, server).await;
  }

  /// Resetting a stream with a numeric code (as a delivery timeout would) and the
  /// peer reading that exact code back. The round-trip of the code is the point.
  async fn reset_roundtrip(client: TransportConnection, server: TransportConnection) {
    use crate::model::error::StreamResetCode;
    let code = StreamResetCode::DeliveryTimeout.to_u64();
    assert_eq!(code, 0x2);

    // Establish the stream first (write + read some data), so the reset that
    // follows isn't racing the stream open.
    let (mut client_send, _client_recv) = client.open_bi().await.expect("client open_bi");
    client_send.write_all(b"partial").await.expect("write_all");
    client_send.flush().await.expect("flush");

    let (_server_send, mut server_recv) = server.accept_bi().await.expect("server accept_bi");
    let mut buf = [0u8; 64];
    let n = server_recv
      .read(&mut buf)
      .await
      .expect("read data")
      .expect("stream open");
    assert_eq!(&buf[..n], b"partial");

    // Now reset with the numeric code; the peer must read it back.
    client_send.reset(code).expect("reset");
    loop {
      match server_recv.read(&mut buf).await {
        Ok(Some(_)) => continue,
        Ok(None) => panic!("expected a stream reset, got a clean FIN"),
        Err(TransportReadError::Reset(read_code)) => {
          assert_eq!(
            read_code, code,
            "peer must read back the numeric reset code"
          );
          break;
        }
        Err(e) => panic!("unexpected read error: {e:?}"),
      }
    }
  }

  #[tokio::test]
  async fn quic_reset_code_round_trips() {
    let (client, server) = quic_pair().await.expect("quic_pair");
    reset_roundtrip(client, server).await;
  }

  #[tokio::test]
  async fn webtransport_reset_code_round_trips() {
    let (client, server) = webtransport_pair().await.expect("webtransport_pair");
    reset_roundtrip(client, server).await;
  }
}
