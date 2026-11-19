---
'moqtail': minor
---

A built-in `webcodecs` transform decodes a `loc` pad into a `raw` pad of `VideoFrame`s or `AudioData`, one pad per instance. Pads carry these as `{ type: 'frame', frame }` items: a `Frame` has the `track`, `location` and `properties` of the object it was decoded from, and the receiver closes its `payload`.
