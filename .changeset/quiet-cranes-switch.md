---
'moqtail-ssts': minor
'relay': patch
---

Move the SSTS bandwidth allocation algorithms out of the relay and into
`moqtail-ssts`, behind a contract that borrows: an algorithm gets an `AbrInput`
(the group, the transport estimate and the operator cap, its switching sets, the
live forwarding streams per set, and the streams reset for delivery timeout since
the previous decision) and returns a `Selection`. It no longer sees the relay's
connection, config or track map, and `decide` is synchronous.

Adding an algorithm is now one directory in the crate and one line in its
registry, which is also what the relay's advertised id list is derived from.

Also fixes the stream accounting the algorithms are fed: slots are released on
the reset and fetch paths that previously leaked them, per-set counts are pruned
when a set is torn down, and a track re-subscribed under a new relay track id
stops counting its old streams. Without these a connection's measured depth
crept up over its lifetime and the backpressure algorithm stayed pinned to the
lowest tier.
