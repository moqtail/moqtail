---
'moqtail-rs': patch
---

qlog files are opened for appending, so logrotate's `copytruncate` can empty a trace the relay is still writing without leaving a run of zero bytes at its start.
