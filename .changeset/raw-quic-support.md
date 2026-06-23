---
'moqtail-rs': minor
'relay': minor
'client': minor
---

Add raw QUIC transport support alongside WebTransport. `TransportConnection`/`TransportSendStream`/`TransportRecvStream` in moqtail-rs now wrap both wtransport and raw quinn behind one API, threaded through the relay and client stream handlers. The relay's QUIC listener demultiplexes WebTransport and raw-QUIC connections on the same UDP socket/port via ALPN, and the client connects over raw QUIC when given a `moqt://authority[/path]` server URL (carrying authority/path via CLIENT_SETUP parameters instead of HTTP CONNECT).
