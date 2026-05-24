---
'relay': patch
---

Fix silent object drops for subscribers joining a track mid-subgroup. New subscribers had no open QUIC send stream for an in-progress subgroup, so objects with no header info were silently discarded. The relay now caches the original subgroup header when the first object of each subgroup arrives and uses it to open a send stream for late joiners. The cache entry is evicted when the publisher unistream closes.
