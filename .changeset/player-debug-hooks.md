---
'moqtail': minor
---

Add what a debugging view needs without reaching into internals. `player.client` is the `MOQtailClient` the player uses, and the new `client` player event fires once it connects and again before each `reconnected`, so an app can listen to `dataevent` from the first stream on. `client.trackNameOf(headerEvent)` names the track a data stream carries, even when the stream opened before the SUBSCRIBE_OK naming its alias; fetch stream header events now carry `fullTrackName` too. `CatalogElement.json` is the catalog as last applied, deltas merged, typed `CatalogJson`.
