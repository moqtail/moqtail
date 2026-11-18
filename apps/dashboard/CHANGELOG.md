# dashboard

## 0.2.0

### Minor Changes

- [#477](https://github.com/moqtail/moqtail/pull/477) [`118c7a7`](https://github.com/moqtail/moqtail/commit/118c7a7088c4407a21abc9c68836fded1d388e5c) Thanks [@zafergurel](https://github.com/zafergurel)! - A web dashboard for one or more relays' `/api/v1`: overview with live charts and the relay's `api.meta` values, connections with their message timeline, tracks with fan-out and cache, latency and slowlog, upstreams, cluster peer links and the routes advertised to each peer, zones, live events, and the operator actions. `appsettings.js` next to `index.html` sets the relays per host, as in the meet app; with `relay` set, the dashboard shows that relay only. A track page plays its namespace's media when the namespace has a catalog track, loading the player only on request; the relay's MoQ address is set per relay in Settings or as `moqUrl` in `appsettings.js`. The message timeline shows data streams cut short, FINs per track, how the connection closed, and when the trace was truncated.
