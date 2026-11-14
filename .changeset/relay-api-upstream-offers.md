---
'relay-api': patch
---

Upstream health counts only the requests that ask a peer for something; a PUBLISH or PUBLISH_NAMESPACE the relay sends, often to a subscriber, no longer shows as an upstream failure or timeout.
