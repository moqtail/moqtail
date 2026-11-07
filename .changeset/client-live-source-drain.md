---
'moqtail': patch
---

`LiveTrackSource` signals done only after its listeners have delivered every object, so a publisher that writes a last object and closes the stream no longer loses it: PUBLISH_DONE now follows the final object instead of cutting it off.
