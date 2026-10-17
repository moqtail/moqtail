# Backpressure tier selection

A bandwidth allocation algorithm that does not use a bandwidth estimate.

It counts forwarding streams instead: how many are still open on this
connection's switching sets, plus how many were reset for delivery timeout
since the last decision. The connection holds a single tier index, all of its
active sets forward the member at that index, and the index moves:

- **up** by one after five consecutive groups at depth 1 or below;
- **down** by one as soon as depth reaches 2, which starts a settle-down period
  during which a stream that times out on the tier just tried, or two further
  groups deeper than the last, drops the tier again. The period ends when depth
  returns to 1.

The reasoning is that a stream the subscriber has not finished consuming is
direct evidence of overrun, while a bandwidth estimate is an inference the
congestion controller makes about the future. The cost is that the signal is
coarse and arrives late: a group is dropped or delayed before the tier moves.

## What it does badly

**Depth is summed across the connection, not measured per set.** One open
stream per set is normal operation, but with two active sets that already reads
as depth 2, which is the downshift threshold. So a connection with two or more
active sets can neither climb (a clear streak requires depth at or below 1) nor
drop below the rung it started on: it stays on the bottom rung no matter how
much capacity is available. `two_active_sets_are_pinned_to_the_lowest_tier` in
`tests.rs` pins this behaviour down. The thresholds were written for a single
set; making them work for several means either measuring depth per set or
scaling them with the number of active sets, and both change the algorithm's
behaviour on links where it currently works. Not done here, deliberately.

**A timed-out stream counts as depth.** Timeouts are added to the open-stream
count, so a burst of resets reads as a burst of queues. That is intended — a
reset means the transport discarded objects it had already committed to — but it
means a transient loss spike pushes the tier down faster than the queue actually
grew.

**One index for all of a connection's sets.** Sets with different ladders share
the index, clamped to each ladder's length, so a set with two rungs and a set
with six both report "the current tier" and get different qualities. The
subscriber perceives all its tracks changing quality together, which is the
point, but the allocation is not per-set.

**No estimate means no floor.** The algorithm has no notion of "this track needs
at least 500 kbps"; it only knows how many streams are outstanding. A set whose
lowest rung is too ambitious will keep forwarding it until streams back up.

## State

Per connection, one `TierState`: the tier index, the clear streak, the depth
recorded at the last downshift (which also means "in cooldown"), and the rising
depth streak. It lives in the algorithm instance the connection owns, so it is
freed with the connection.
