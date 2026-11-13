---
'moqtail-rs': minor
'relay': minor
'client': minor
---

qlog traces now also record a `moqt:subgroup_header_*`/`moqt:fetch_header_*` event for every data stream opened or accepted (one per stream, not per object). `scripts/qlog-summary.py <folder>` prints each connection's peer SETUP, requests with their responses, data streams per track and parse errors.
