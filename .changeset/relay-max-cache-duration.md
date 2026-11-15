---
'relay': patch
---

The relay now honors the MAX_CACHE_DURATION track property. A cached group expires that long after its first Object arrived, so a FETCH no longer receives Objects past their cache lifetime; it used to keep them until the relay's own cache expiry evicted them.
