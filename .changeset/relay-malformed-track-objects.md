---
'relay': patch
'moqtail-rs': minor
'client': patch
---

An Object carrying a Mandatory Track Property, other than a GREASE value, now makes its track malformed instead of being forwarded. The relay ends downstream subscriptions with PUBLISH_DONE MALFORMED_TRACK and cancels the upstream request, whether it subscribed to the track or the publisher pushed it with PUBLISH, for subgroup streams and datagrams alike, withdraws a pushed track's route from cluster peers, and refuses a later PUBLISH of that track from the same session with MALFORMED_TRACK. `ParseError::MalformedTrack` marks such errors. A GREASE value in the Mandatory Track Property range is an ordinary unknown property, on a track as on an Object, so a PUBLISH, SUBSCRIBE_OK or FETCH_OK carrying one is no longer refused. The test client still sends a property in that range when `--object-property` asks for one. The test client stops delivering when its PUBLISH or a SUBSCRIBE it serves is cancelled by a stream reset, and keeps delivering through a FIN or a REQUEST_UPDATE; after a FIN it watches for STOP_SENDING, which `ControlStreamHandler::stopped` now reports.
