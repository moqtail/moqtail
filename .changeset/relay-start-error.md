---
'relay': patch
---

The relay now exits with a failure status and logs the reason when it cannot start, such as when its port is already in use, instead of exiting silently with status 0.
