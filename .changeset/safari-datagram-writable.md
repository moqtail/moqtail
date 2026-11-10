---
'moqtail': patch
---

Datagrams work on Safari: when the transport has no `datagrams.writable` (Safari implements only `datagrams.createWritable()`), the client sends through `createWritable()` instead of failing to connect.
