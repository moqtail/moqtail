---
'moqtail-rs': patch
'relay': patch
---

Accept FILL_PARAMETERS whose parameters have no leading count, as some peers send, instead of closing the session with PROTOCOL_VIOLATION. FILL_PARAMETERS is still sent with a count.
