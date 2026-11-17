---
'moqtail-rs': patch
---

A data stream that goes quiet past its timeout is logged once at INFO ("Data stream closed after 15s without data") instead of twice at ERROR under a "header_read" label, which also applied to read errors anywhere in the stream.
