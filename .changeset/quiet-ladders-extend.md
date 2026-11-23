---
'moqtail': minor
---

`SwitchingSetAssignment` carries the draft's latency-budget extension:
`subscriberBufferMs` and the algorithm's own varints are decoded and encoded
for non-zero algorithms, and `ssts({ subscriberBufferMs, algorithmParams })`
sends them with every SUBSCRIBE. Algorithm 0 stays byte-identical to the six
base fields.
