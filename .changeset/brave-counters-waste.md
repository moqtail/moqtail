---
'relay': minor
---

Count and log what a stream reset wastes. The relay now tracks the bytes
written to each live send stream; when a stream is reset mid-delivery —
a supersede from a retroactive downshift, a discard timeout, or a
teardown — everything already sent on it is data the subscriber will
never use, so `MOQTClient::reset_stream` and the timeout arm of
`close_stream` log one line per reset naming the reason
(`cancelled` / `delivery-timeout` / `other`) and the `wasted_bytes`. A
graceful finish clears the ledger entry without a log: a delivery that
completed wasted nothing. The testbed sums the lines and reports the
run's total.
