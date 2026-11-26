---
'moqtail-ssts': minor
'relay': patch
---

Unify the backpressure family under one id, `0xff04`. The retroactive supersede
is now the fourth assignment-extension parameter (zero or absent = classic,
non-zero = cancel and re-send the newest overtaken group) instead of the
separate `0xff05` / `0xff06` ids. The retired ASD v1.1 and the staged v3
behaviour are removed; ASD remains as the legacy comparison.

A supersede now leaves the old stream alone when the replacement cache has no
beginning to replay. Other switching sets may share the connection as long as
they are not assigned to backpressure; the depth and its climb guards are
scoped to the one backpressure set.
