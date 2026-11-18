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

//! Our QMux against moq-dev's independent implementation, both ways round. Agreeing
//! on bytes with a second implementation is what pins the provisional codepoints and
//! the parts the specification leaves open.

use bytes::Bytes;
use moqtail::transport::connection::{CloseReason, TransportReadError};
use moqtail::transport::qmux::{QmuxConfig, QmuxSession, Role, websocket_config};
use qmux::ws::tokio_tungstenite::WebSocketStream;
use tokio::net::{TcpListener, TcpStream};
use web_transport_trait::{Error as _, RecvStream as _, SendStream as _, Session as _};

/// The subprotocol moq-dev negotiates: QMux version, then the application protocol.
const SUBPROTOCOL: &str = "qmux-02.moqt-22";

async fn tcp_pair() -> (TcpStream, TcpStream) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (client, server) = tokio::join!(TcpStream::connect(addr), listener.accept());
  (client.unwrap(), server.unwrap().0)
}

/// Their side on one end of a TCP pair. The HTTP upgrade is skipped on both sides: it
/// only settles the subprotocol, which is passed in directly.
async fn theirs(tcp: TcpStream, role: Role) -> qmux::Session {
  let ws = WebSocketStream::from_raw_socket(tcp, role, None).await;
  let upgraded = qmux::ws::Upgraded::new(ws).with_alpn(SUBPROTOCOL);
  match role {
    Role::Client => upgraded.connect(),
    Role::Server => upgraded.accept(),
  }
}

async fn ours(tcp: TcpStream, role: Role) -> QmuxSession {
  let remote = tcp.peer_addr().unwrap();
  let ws = WebSocketStream::from_raw_socket(tcp, role, Some(websocket_config())).await;
  QmuxSession::new(ws, role, remote, QmuxConfig::default())
    .await
    .expect("QMux handshake with moq-dev")
}

async fn read_to_end(recv: &mut moqtail::transport::qmux::QmuxRecvStream) -> Vec<u8> {
  let mut out = Vec::new();
  let mut buf = [0u8; 4096];
  while let Some(n) = recv.read(&mut buf).await.unwrap() {
    out.extend_from_slice(&buf[..n]);
  }
  out
}

#[tokio::test]
async fn our_client_with_their_server() {
  let (client_tcp, server_tcp) = tcp_pair().await;
  let (server, client) = tokio::join!(
    theirs(server_tcp, Role::Server),
    ours(client_tcp, Role::Client)
  );

  // Bidirectional: we write, they echo back in upper case.
  let echo = tokio::spawn({
    let server = server.clone();
    async move {
      let (mut send, mut recv) = server.accept_bi().await.unwrap();
      let data = recv.read_all().await.unwrap();
      send.write_all(&data.to_ascii_uppercase()).await.unwrap();
      send.finish().unwrap();
    }
  });
  let (mut send, mut recv) = client.open_bi().await.unwrap();
  let big = vec![b'a'; 3 << 20]; // past both peers' initial windows
  send.write_all(&big).await.unwrap();
  send.finish().unwrap();
  assert_eq!(read_to_end(&mut recv).await, vec![b'A'; 3 << 20]);
  echo.await.unwrap();

  // Unidirectional from them.
  let mut their_uni = server.open_uni().await.unwrap();
  their_uni.write_all(b"from moq-dev").await.unwrap();
  their_uni.finish().unwrap();
  let mut our_recv = client.accept_uni().await.unwrap();
  assert_eq!(read_to_end(&mut our_recv).await, b"from moq-dev");

  // Our reset code reaches them unchanged.
  let mut ours_uni = client.open_uni().await.unwrap();
  ours_uni.write_all(b"partial").await.unwrap();
  ours_uni.flush().await.unwrap();
  let mut their_recv = server.accept_uni().await.unwrap();
  ours_uni.reset(0x2).unwrap();
  let err = their_recv.read_all().await.unwrap_err();
  assert_eq!(err.stream_error(), Some(0x2), "{err}");

  // Datagrams both ways.
  client.send_datagram(Bytes::from_static(b"ours")).unwrap();
  assert_eq!(server.recv_datagram().await.unwrap(), "ours");
  server.send_datagram(Bytes::from_static(b"theirs")).unwrap();
  assert_eq!(client.receive_datagram().await.unwrap(), "theirs");

  // Our close code and reason reach them.
  client.close(0x10, b"bye");
  let err = server.closed().await;
  assert_eq!(
    err.session_error(),
    Some((0x10, "bye".to_string())),
    "{err}"
  );
}

#[tokio::test]
async fn their_client_with_our_server() {
  let (client_tcp, server_tcp) = tcp_pair().await;
  let (client, server) = tokio::join!(
    theirs(client_tcp, Role::Client),
    ours(server_tcp, Role::Server)
  );

  let echo = tokio::spawn({
    let server = server.clone();
    async move {
      let (mut send, mut recv) = server.accept_bi().await.unwrap();
      let data = read_to_end(&mut recv).await;
      send.write_all(&data).await.unwrap();
      send.finish().unwrap();
    }
  });
  let (mut send, mut recv) = client.open_bi().await.unwrap();
  let payload = vec![7u8; 2 << 20];
  send.write_all(&payload).await.unwrap();
  send.finish().unwrap();
  assert_eq!(recv.read_all().await.unwrap(), payload);
  echo.await.unwrap();

  // Their reset code reaches us unchanged.
  let mut their_uni = client.open_uni().await.unwrap();
  their_uni.write_all(b"x").await.unwrap();
  let mut our_recv = server.accept_uni().await.unwrap();
  their_uni.reset(0x2);
  let mut buf = [0u8; 8];
  let err = loop {
    match our_recv.read(&mut buf).await {
      Ok(Some(_)) => continue,
      Ok(None) => panic!("expected a reset, got FIN"),
      Err(e) => break e,
    }
  };
  assert!(matches!(err, TransportReadError::Reset(0x2)), "{err:?}");

  // Their close code and reason reach us.
  client.close(0x10, "bye");
  server.closed().await;
  assert_eq!(
    server.close_reason(),
    Some(CloseReason::ByPeer {
      code: 0x10,
      reason: "bye".to_string()
    })
  );
}
