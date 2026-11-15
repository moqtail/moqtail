---
'relay': patch
---

The relay now honors the MAX_CACHE_DURATION track property, from PUBLISH, SUBSCRIBE_OK, or FETCH_OK for a track it only holds in cache. A cached group is not served once that long has passed since its first Object arrived, even when the property arrives after the group; it used to be kept until the relay's own cache expiry evicted it.
