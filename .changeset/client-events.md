---
'moqtail': minor
'client-js': patch
'meet': patch
---

Breaking: `MOQtailClient` events now take any number of listeners. `new MOQtailClient(options)` does no I/O and `await client.init()` connects, so listeners attached with `client.on(event, listener)` see the handshake too; `client.off()` removes one. `MOQtailClient.new()`, the `callbacks` option and every `on*` property are removed: `client.onPeerPublish = fn` becomes `client.on('peerpublish', fn)`. `client.subscribe` returns `{ requestId, subscribeOk, stream, done }`, where `done` resolves with the PUBLISH_DONE that ended the subscription, and `client.fetch` returns `{ requestId, fetchOk, stream }`. `findProperty(properties, PropertyClass)` reads a typed property out of a track or object property list. `Emitter` is exported from `moqtail/util`. The stream classes (`SendStream`, `RecvStream`, `RequestStream`, the control and datagram streams) take the client's emitter instead of `on*` callbacks, and `DataEventObserver` and `DataStreamObserver` are removed.
