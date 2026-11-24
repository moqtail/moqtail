---
'relay': patch
---

The SSTS discard timer is now exclusively a switching-set property. A track in
no switching set has no timer, as does a set on the default algorithm or a set
that negotiated a zero buffer. The connection-wide `ssts_discard_timeout`
setting is removed.
