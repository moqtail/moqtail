---
'relay': minor
'client': minor
---

The relay accepts MoQ over WebSocket (QMux) for clients that can't use QUIC. Set `websocket_bind` (usually the same address as `bind`; TCP and UDP ports don't clash) to turn it on; it is off by default. The WebSocket subprotocol names the MoQ version, as `qmux-02.moqt-22` or a bare `moqt-22`. The CLI client connects this way with `--transport websocket`.
