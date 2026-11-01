---
'relay': minor
'moqtail-rs': minor
'client': minor
'moqtail': minor
---

Serve FETCH from the relay's cache whenever the cache accounts for every Object of a group, and fetch upstream only what it does not. The relay now creates a track for a FETCH of a track it does not have yet, keeps a track for its cached Objects after its upstream goes away, records which Objects are known not to exist (from End of Group and End of Track statuses, gap properties, a group's last datagram or subgroup stream, and what an upstream FETCH response passes over), and lets concurrent FETCHes of the same range share one upstream FETCH. A PUBLISH_DONE now waits for a track's datagrams to go quiet. Both libraries parse End of Timed-Out Range markers and hand End of Range markers to the application (`RecvDataStream::next_item` in moqtail-rs, an `onEndOfRange` handler in moqtail). The test client can publish groups that already exist at startup, reset streams, drop datagrams, send End of Group and End of Track objects, and cut or mark its FETCH responses.
