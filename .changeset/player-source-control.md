---
'moqtail': minor
---

The CMAF source seeks, switches, and plays URL ranges. A seek is answered with one `discontinuity`, also when superseded, failed, or across a reconnect: VOD with FETCH from the containing group, live with one filled SUBSCRIBE behind the live edge or a SUBSCRIBE from the next group at it, placed by the media timeline or the observed group duration and clamped to the accessible window. `switch()` works live (SUBSCRIBE plus REQUEST_UPDATE, or `SWITCH_FROM` with the `switchFrom` option) and for VOD, at MSF 6.3.2 switch points when the set has `keyframeSpacing`, and resolves after the new track's first packet; concurrent calls reject with `ABORTED`. An empty `selectRenditions()` stops requesting until a later selection, and `setTracks()` switches away from a removed active track. URL ranges are merged, sorted, played with gaps between them, and end with one `eos`. `renditionchange` is emitted when the track changes, not for the first one.
