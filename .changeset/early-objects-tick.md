---
'moqtail-ssts': minor
'relay': minor
---

The mid-group sample clock now fires when the delivery starts an Object,
not when it finishes one: the tick moved ahead of the blocking QUIC write
in `write_stream_object`, so a sample is no longer held up by the very
backpressure it exists to measure. `ObjectSent` carries the group and
track it went out on, and the controller ticks only for the newest
group's fresh delivery: tail Objects draining on an overtaken group and
the replay of a re-sent group are bytes, not progress. A write whose
stream left the send map - cancelled by a supersede between objects -
aborts instead of putting the old tier's Object into a just-cancelled
stream. In the state machine, boundary detection is strictly monotonic:
a reading naming an older group is a mid-group reading, never a second
boundary, and it cannot move the boundary clock back.
