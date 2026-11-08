---
'moqtail': patch
---

A publication no longer ends when the peer stops one of its subgroup streams (STOP_SENDING). The rest of that group is skipped and later groups are still sent.
