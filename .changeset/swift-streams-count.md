---
'moqtail': minor
---

Add `onDataEvent`, which reports every data stream as lightweight records: its header when it opens, each object's IDs and payload length, and how it ended (fin, reset with its code, or error). Each record carries the local time it happened, and object records carry the LOC Timestamp property when present, for interarrival, jitter and latency measurements. Payloads are never handed over. It can be passed in `MOQtailClient.new({ callbacks })`.
