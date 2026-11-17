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

//! Runs a QMux session over a tokio-tungstenite connection: one binary message per
//! record, WebSocket Ping/Pong for keepalive and RTT. Every tungstenite call lives in
//! this file.

use super::codec::{ProtocolError, code};
use super::session::{Handle, QmuxConfig, QmuxSession, Shared};
use crate::transport::connection::{CloseReason, TransportConnectionError};
use bytes::Bytes;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{self, Message};

// Short in tests so keepalive is exercised without slowing the suite.
const PING_INTERVAL: Duration = if cfg!(test) {
  Duration::from_millis(50)
} else {
  Duration::from_secs(5)
};
/// How long to wait for the peer's transport parameters.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// WebSocket-level cap; records are checked against max_record_size after this.
const MAX_MESSAGE_SIZE: usize = 2 << 20;
/// Records are buffered and flushed once the writer has nothing more to send. Batching
/// writes measured about a third more throughput than one write per message.
const WRITE_BUFFER: usize = 128 * 1024;

/// The WebSocket settings a QMux session expects; use them when accepting or opening
/// the WebSocket. They cap messages near the largest record, so a peer can't make us
/// buffer a huge one, and batch writes until the writer flushes.
pub fn websocket_config() -> WebSocketConfig {
  WebSocketConfig::default()
    .max_message_size(Some(MAX_MESSAGE_SIZE))
    .max_frame_size(Some(MAX_MESSAGE_SIZE))
    .write_buffer_size(WRITE_BUFFER)
}

impl QmuxSession {
  /// Starts a session on a WebSocket whose HTTP upgrade is done, and returns once the
  /// peer's transport parameters have arrived. `role` is this endpoint's side.
  pub async fn new<S>(
    ws: WebSocketStream<S>,
    role: Role,
    remote: SocketAddr,
    config: QmuxConfig,
  ) -> Result<Self, TransportConnectionError>
  where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
  {
    let (sink, stream) = ws.split();
    let shared = Arc::new(Shared::new(role == Role::Client, &config, remote));
    tokio::spawn(write_loop(sink, shared.clone()));
    tokio::spawn(read_loop(stream, shared.clone()));
    shared.writer.notify_one();

    let handshake = shared.wait(shared.ready(), |s| {
      if s.peer.is_some() {
        Some(Ok(()))
      } else {
        s.close.is_some().then(|| Err(s.closed_error()))
      }
    });
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
      Ok(Ok(())) => Ok(Self {
        shared: Arc::new(Handle(shared)),
      }),
      Ok(Err(e)) => Err(e),
      Err(_) => {
        shared.with(|s| s.lost(CloseReason::IdleTimeout));
        Err(TransportConnectionError::Other(
          "timed out waiting for the QMux transport parameters".to_string(),
        ))
      }
    }
  }
}

/// tungstenite answers Ping and Close by itself, on the next read or write.
async fn read_loop<S>(mut ws: SplitStream<WebSocketStream<S>>, shared: Arc<Shared>)
where
  S: AsyncRead + AsyncWrite + Unpin,
{
  loop {
    let closed = shared.closed.notified();
    if shared.with(|s| s.close.is_some()) {
      break;
    }
    let message = tokio::select! {
      message = ws.next() => message,
      _ = closed => break,
    };
    match message {
      Some(Ok(Message::Binary(record))) => shared.with(|s| s.on_record(record)),
      Some(Ok(Message::Text(_))) => {
        shared.with(|s| s.fail(ProtocolError::new(code::PROTOCOL_VIOLATION, "text message")));
      }
      Some(Ok(Message::Close(_))) => {
        // A peer that closes without CONNECTION_CLOSE still closed on purpose.
        shared.with(|s| {
          s.lost(CloseReason::ByPeer {
            code: 0,
            reason: String::new(),
          })
        });
        break;
      }
      Some(Ok(Message::Pong(payload))) => shared.with(|s| s.on_pong(&payload)),
      Some(Ok(_)) => shared.with(|s| s.touch()),
      Some(Err(e)) => {
        shared.with(|s| s.lost(CloseReason::Other(format!("WebSocket read: {e}"))));
        break;
      }
      None => {
        shared.with(|s| s.lost(CloseReason::Reset));
        break;
      }
    }
  }
}

async fn write_loop<S>(mut ws: SplitSink<WebSocketStream<S>, Message>, shared: Arc<Shared>)
where
  S: AsyncRead + AsyncWrite + Unpin,
{
  let mut keepalive = tokio::time::interval(PING_INTERVAL);
  keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
  keepalive.tick().await;

  let result: Result<(), tungstenite::Error> = async {
    loop {
      while let Some(out) = shared.with(|s| s.build_record()) {
        let len = out.record.len() as u64;
        ws.feed(Message::Binary(Bytes::from(out.record))).await?;
        shared.with(|s| {
          s.stats.sent_bytes += len;
          s.stats.sent_packets += 1;
        });
        if out.last {
          return ws.send(Message::Close(None)).await;
        }
      }
      ws.flush().await?;
      if shared.with(|s| s.close.is_some()) {
        // Closed by the peer or by the network; a Close frame may no longer get out.
        let _ = ws.send(Message::Close(None)).await;
        return Ok(());
      }
      tokio::select! {
        _ = shared.writer.notified() => {}
        _ = keepalive.tick() => {
          if let Some(seq) = shared.with(|s| s.keepalive()) {
            let ping = Bytes::copy_from_slice(&seq.to_be_bytes());
            ws.feed(Message::Ping(ping)).await?;
          }
        }
      }
    }
  }
  .await;
  if let Err(e) = result {
    shared.with(|s| s.lost(CloseReason::Other(format!("WebSocket write: {e}"))));
  }
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use tokio::net::{TcpListener, TcpStream};

  /// A client and server session over loopback TCP. The HTTP upgrade is skipped: QMux
  /// starts after it, so it plays no part in what these tests check.
  pub(crate) async fn pair() -> (QmuxSession, QmuxSession) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
      let (tcp, remote) = listener.accept().await.unwrap();
      let ws = WebSocketStream::from_raw_socket(tcp, Role::Server, Some(websocket_config())).await;
      QmuxSession::new(ws, Role::Server, remote, QmuxConfig::default())
        .await
        .unwrap()
    });
    let tcp = TcpStream::connect(addr).await.unwrap();
    let ws = WebSocketStream::from_raw_socket(tcp, Role::Client, Some(websocket_config())).await;
    let client = QmuxSession::new(ws, Role::Client, addr, QmuxConfig::default())
      .await
      .unwrap();
    (client, server.await.unwrap())
  }

  #[tokio::test]
  async fn many_streams_both_ways_and_a_datagram() {
    let (client, server) = pair().await;
    let echo = tokio::spawn({
      let server = server.clone();
      async move {
        for _ in 0..20 {
          let (mut send, mut recv) = server.accept_bi().await.unwrap();
          tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            while let Some(n) = recv.read(&mut buf).await.unwrap() {
              send.write_all(&buf[..n]).await.unwrap();
            }
            send.finish().unwrap();
          });
        }
      }
    });
    let mut tasks = Vec::new();
    for i in 0..20u8 {
      let client = client.clone();
      tasks.push(tokio::spawn(async move {
        let (mut send, mut recv) = client.open_bi().await.unwrap();
        let payload = vec![i; 200_000];
        let writer = tokio::spawn(async move {
          send.write_all(&payload).await.unwrap();
          send.finish().unwrap();
        });
        let mut got = Vec::new();
        let mut buf = vec![0u8; 8192];
        while let Some(n) = recv.read(&mut buf).await.unwrap() {
          got.extend_from_slice(&buf[..n]);
        }
        writer.await.unwrap();
        assert_eq!(got, vec![i; 200_000]);
      }));
    }
    for task in tasks {
      task.await.unwrap();
    }
    echo.await.unwrap();

    client.send_datagram(Bytes::from_static(b"dgram")).unwrap();
    assert_eq!(server.receive_datagram().await.unwrap(), "dgram");
  }

  #[tokio::test]
  async fn keepalive_pings_measure_rtt() {
    let (client, server) = pair().await;
    tokio::time::sleep(PING_INTERVAL * 4).await;
    assert!(client.transport_stats().rtt.is_some(), "client got no pong");
    assert!(server.transport_stats().rtt.is_some(), "server got no pong");
    assert_eq!(client.close_reason(), None);
  }

  #[tokio::test]
  async fn dropping_every_handle_closes_the_connection() {
    let (client, server) = pair().await;
    let send = client.open_uni().await.unwrap();
    drop(client);
    // A stream still holds the connection open.
    tokio::task::yield_now().await;
    assert_eq!(server.close_reason(), None);
    drop(send);
    server.closed().await;
    assert_eq!(
      server.close_reason(),
      Some(CloseReason::ByPeer {
        code: 0,
        reason: String::new()
      })
    );
  }

  #[tokio::test]
  async fn close_wakes_everyone_with_the_reason() {
    let (client, server) = pair().await;
    let pending = tokio::spawn({
      let server = server.clone();
      async move { server.accept_uni().await }
    });
    client.close(0x10, b"bye");
    server.closed().await;
    assert!(matches!(
      pending.await.unwrap(),
      Err(TransportConnectionError::ApplicationClosed)
    ));
    assert_eq!(
      server.close_reason(),
      Some(CloseReason::ByPeer {
        code: 0x10,
        reason: "bye".to_string()
      })
    );
  }
}
