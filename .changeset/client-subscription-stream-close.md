---
'moqtail': patch
---

A subscription's stream ends on `unsubscribe()` and when the session ends, so a loop reading it finishes instead of waiting forever.
