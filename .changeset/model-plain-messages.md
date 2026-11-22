---
'moqtail': minor
'client-js': patch
---

Control messages, message parameters and setup options are plain data, coded by one table-driven codec: `MOQtailPlayer` is about 6 kB gzip smaller.

- Messages are objects tagged by `kind` (`{ kind: 'subscribe', requestId, fullTrackName, parameters }`) instead of classes. Test a request's result with `isRequestError(result)` instead of `instanceof RequestError`. `reasonPhrase` and `errorReason` are strings.
- `MessageParameters` is an object keyed by parameter (`{ forward: true, fillParameters: { locationFilter } }`) instead of an array of parameter classes. `subscribe`, `fetch`, `subscribeUpdate`, `publish` and the namespace calls take it as `parameters`.
- `SetupOptions` is an object (`{ maxFilterRanges: 8n, moqtImplementation: 'app' }`) instead of a builder. Options of other types go in `extensions`.
- `locationFilter(type, …)` builds a `LocationFilter`; `startLocation`, `endLocation`, `absoluteStart` and `fetchRange` are functions of one.
- `ObjectDatagramType` and `SubgroupHeaderType` are numbers; their bit helpers are unchanged.
