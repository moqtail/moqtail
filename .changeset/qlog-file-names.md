---
'moqtail-rs': patch
'relay': patch
---

The relay names its qlog files by connection id alone (`server_conn<id>.sqlog`) and never overwrites an earlier trace: an id seen again after a restart gets a `_2`, `_3`, … suffix.
