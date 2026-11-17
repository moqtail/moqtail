---
'moqtail-rs': minor
---

Add a WebSocket transport: `TransportConnection::WebSocket` runs MoQ over QMux (QUIC's streams, flow control and datagrams carried in WebSocket binary messages) on a tokio-tungstenite connection, for clients that can't reach a relay over QUIC. Open or accept the WebSocket with `qmux::websocket_config()`, then start a session with `QmuxSession::new`. It interoperates with moq-dev's `qmux` implementation. `TransportKind::WebSocket` gets the WebTransport SETUP rules (no AUTHORITY or PATH options).
