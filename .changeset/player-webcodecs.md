---
'moqtail': minor
---

A built-in `webcodecs` transform decodes a `loc` pad into a `raw` pad of `VideoFrame`s or `AudioData`, one pad per instance. Pads carry these as `{ type: 'frame', frame }` items: a `Frame` has the `track`, `location` and `properties` of the object it was decoded from, and the receiver closes its `payload`. It decodes only once the consumer took the frame before: a frame's write resolves once it's taken, so packets wait encoded in the transform rather than as frames pinning decoder buffers. It sends the seconds it holds downstream as a `{ type: 'held', seconds }` pad signal whenever they change, and holds the source back past 10 s.
