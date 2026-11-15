---
'moqtail': patch
---

Reject received Objects that carry a Mandatory Track Property (other than a GREASE value), directly or inside Immutable Properties, or whose property block is malformed. A truncated property block is now a protocol violation instead of stalling the data stream while it waits for bytes that will never come.
