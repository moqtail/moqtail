---
'relay': patch
---

When a subscription's `forward` parameter transitions from false to true via REQUEST_UPDATE mid-group, the relay now immediately opens a QUIC send stream for the subscriber using the subgroup header that arrived while forwarding was paused. Previously, if no new group boundary arrived after the toggle, the subscriber would miss the remainder of the current subgroup.
