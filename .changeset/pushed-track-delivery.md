---
'moqtail': patch
---

Deliver the objects of tracks the peer pushes with PUBLISH again; their data streams failed with "Cannot read properties of undefined (reading 'objects')". A pushed track's stream now also ends on its PUBLISH_DONE. `allocatePseudoRequestId()` is removed.
