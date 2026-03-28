---
'relay': patch
---

Fixed the tracking state of the control messages. They were being deleted from the tracking map too early. Now they are only being deleted after their lifetime is over.
