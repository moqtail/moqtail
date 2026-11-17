---
'moqtail': patch
'meet': patch
---

A publication now closes a group's stream as soon as it writes an end-of-group or end-of-track Object, instead of when the next group starts. A quiet track, such as meet's signalling, no longer leaves its last stream idle until the relay times it out. Meet marks each signalling message's group as ended.
