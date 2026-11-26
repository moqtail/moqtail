---
'moqtail': minor
---

The TypeScript client and player can encode, decode and send the SSTS
switching-set assignment extension. The `ssts()` options `subscriberBufferMs`
and `algorithmParams` carry the subscriber's latency budget and the selected
algorithm's parameters; algorithm 0 keeps the six base fields byte-identical.
