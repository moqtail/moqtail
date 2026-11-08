---
'moqtail': minor
---

Breaking: `MOQtailClient` events take any number of listeners through `client.on(event, listener)` and `client.off()`. `new MOQtailClient(options)` does no I/O and `await client.init()` connects; `MOQtailClient.new()`, the `callbacks` option and the `on*` properties are removed. `client.subscribe` returns `{ requestId, subscribeOk, stream, done }` and `client.fetch` returns `{ requestId, fetchOk, stream }`.

- An `ordering` option on the client, `subscribe` and `fetch` releases objects in location order across streams and datagrams, reporting what it stopped waiting for with `orderinggap`.
- `client.switch` returns the new `SubscribeResult` and no longer sends FORWARD; `subscribeUpdate` only sends priority and forward when given.
- `client.trackNameOf(headerEvent)` names the track a data stream carries.
- `client.clientSetup` holds the SETUP the client sent, next to `client.serverSetup`.
- `LiveTrackSource` no longer loses the last object before PUBLISH_DONE, and a closing session leaves no unhandled rejections.
- `moqt://` is matched case-insensitively.
