---
'relay': patch
---

Fix relay not sending FETCH_OK for non-empty fetch ranges. The relay previously only sent FETCH_OK when the requested range was empty; for any fetch that returned objects the control-stream response was omitted, causing subscribers to block indefinitely waiting for it.
