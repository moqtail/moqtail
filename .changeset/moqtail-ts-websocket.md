---
'moqtail': minor
---

`MOQtailClient` can run MoQ over WebSocket (QMux) when WebTransport isn't available or can't connect, for example where UDP is blocked or in Node.js. The new `transport` option is `auto` by default: WebTransport when the runtime has it and it connects within `webTransportTimeoutMs` (3 s), otherwise WebSocket. `webtransport` and `websocket` pin one. `client.transportKind` says which was used, and `client.webTransport` is now typed as `MoqtTransport`, the WebTransport members the client uses. `<moqtail-video>` gets the fallback with no configuration. The relay needs its WebSocket listener on (`websocket_bind`).
