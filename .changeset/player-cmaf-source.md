---
'moqtail': minor
---

The player gets a built-in CMAF media source, reached through the probe like any built-in source. It plays a CMAF switching set on one pad: live with SUBSCRIBE from the next group, VOD with FETCH, init data from the catalog, a track property, or object properties, and `eos` on PUBLISH_DONE TRACK_ENDED or the end of a FETCH, held until the catalog confirms the end. Rejected renditions fall back to the closest bitrate, failed streams retry with backoff, and a lost session resumes once the client reconnects. `moqtail/player` exports `MediaSourceOptions`, the load options every media source takes, and `CatalogReader` gains `url` and `namespace`.
