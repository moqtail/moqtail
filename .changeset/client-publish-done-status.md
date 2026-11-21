---
'moqtail': patch
---

`publishDone()` reports `TRACK_ENDED` when no status code is given, matching its default reason phrase. It reported `INTERNAL_ERROR`.
