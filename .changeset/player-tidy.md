---
'moqtail': patch
---

Removes unused API. `PlayerError` takes no `category` option, since the category always follows from the code. `ElementDefinition` loses the unused `description`. `moqtail/client` no longer exports `RecvDatagramStream`, which nothing used. `ObjectOrderer.suspendWindow(suspended, reason)` is now `suspendWindow(reason)` and `resumeWindow(reason)`, and the orderer's `ordering`, `release`, and `gap` fields are private.
