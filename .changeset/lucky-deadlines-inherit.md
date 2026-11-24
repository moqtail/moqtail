---
'moqtail-ssts': minor
---

BACKPRESSURE's re-send deadline is inherited, not halved. A re-sent group now gets
the remaining buffer budget of the newest overtaken stream it replaces
(`subscriber_buffer_ms - age_ms`) instead of a hardcoded half of the
negotiated buffer: the overtaken stream has already spent part of the
subscriber's delivery window on the wire, and what it has left is exactly
what its replacement has left. A set that negotiated no buffer still asks
for no deadline; an overtaken stream already past the buffer leaves the
re-send expired on arrival (1 ms, since 0 means "never" in the contract).
Applies to both the classic and retroactive modes — it changes only what the
deadline is, not how far the downshift reaches.
