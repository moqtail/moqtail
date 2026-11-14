---
'moqtail-rs': minor
'relay': minor
'dashboard': patch
---

`qlog_max_file_bytes` stops a connection's qlog file at about that size, with a last `moqtail:trace_truncated` event; 0, the default, means no limit. A file emptied in place by logrotate's `copytruncate` takes events again. `qlog::init` takes the limit as a third argument. The dashboard's message timeline notes a trace that was cut.
