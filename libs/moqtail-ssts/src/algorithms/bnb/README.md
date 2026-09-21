# Budget-negotiated backpressure (BNB)

The depth algorithms count streams, which is a quantity without a unit. BNB puts
a unit on it: the subscriber's own reorder buffer, negotiated in the
SWITCHING_SET_ASSIGNMENT extension, is how long that subscriber is willing to
wait, and the set's backlog is the age of its oldest open stream beyond one group
period. The set reacts to the _ratio_.

```
backlog = oldest open stream age - observed group period
budget  = the subscriber's reorder buffer for this set

backlog >  danger_factor * budget   -> drop a rung (a discard is imminent)
backlog >= safe_factor    * budget   -> drop only while streams are accumulating
backlog <  safe_factor    * budget   -> climb a rung after a flat-queue streak
```

Each **switching set has its own rung index, its own streaks and its own budget**,
so a subscriber with an audio ladder and three video ladders sees each of them
move on its own evidence — which is the main difference from
[`../backpressure`](../backpressure), whose single index moves all of them
together.

Timeouts are a separate channel rather than an addition to the queue count: a
burst of them sets a veto that blocks climbs and decays one per clean group, and
a burst that keeps coming drops a rung on its own even when the queue looks
drained. A drain is exactly what hides an unsustainable rung from every other
signal, so this channel exists to not be hidden.

After any change the rung **holds for `drain_window_ms`** (1.5 s by default): the
old rung's streams keep draining for about a buffer's worth of time, so a sample
taken just after a move measures the drain rather than the new state. Holding
every choice for at least the subscriber's buffer also means the choice outlives
the buffer it was made for, which is what keeps the subscriber's playout from
seeing two quality changes inside one buffer.

## What it needs from the wire

`subscriber_buffer_ms` in the assignment's extension, which is the `budget`
above. The relay also honours it as the discard timeout for that set's streams,
so the number means one thing to both ends.

**A subscriber that negotiates nothing leaves the danger channel dead.** With a
zero budget there is no ratio to be dangerous at: the algorithm still climbs on
its flat-queue streaks and still reacts to timeouts, but it has no signal at all
from queue growth, and will climb to the top rung of a link that cannot carry it
and stay there until the transport starts resetting streams. That is a correct
reading of "the subscriber told me it has nowhere to put late media and I may
never discard", but it makes BNB on a client that does not send the extension
behave like an algorithm with no congestion signal. Do not run it against such a
client.

It also needs the host's **group period** measurement, which exists once two
consecutive groups of that set have been seen. Before that every stream age is
backlog, which biases the algorithm conservative: it starts low and may drop a
rung early on a long-group-period link. That is the safe direction, but it is a
real cost on the first seconds of a connection.

## What it does badly

**No cross-set budget.** Each set climbs as far as its own queue allows, and
nothing bounds the sum of the rungs a subscriber is being served. Two sets on one
connection can each decide the link has room for them, and the link will disagree.
The depth algorithms get this right by accident — one shared index is one shared
budget — and BNB gives it up to make per-set evidence meaningful. A correct fix
needs a notion of the connection's share, which is the thing this algorithm was
written to stop guessing at.

**The thresholds are ratios of a number the subscriber guesses.** A subscriber
that reports a 4000 ms buffer it does not really have (a player that reserves
memory rather than media time) makes every threshold proportionally laxer, and
the algorithm will happily run four times the backlog the link can absorb before
it calls it danger. The negotiation is trust, and this algorithm spends that
trust directly.

**The drain window is a fixed hold, not a condition.** For 1.5 s after a change
the rung will not move for anything except sustained timeouts — including a real
collapse that happens the moment after the change. A backpressure-style depth
signal would catch it sooner, which is why BNB's behaviour during a window looks
indifferent if you plot it at 10 Hz.

**One set's evidence can starve another's.** Because there is no cross-set
budget, a set whose subscriber paused (and so has an empty queue) climbs while
the sets actually being watched drop. Paused sets forward nothing, so this is
mostly wasted state rather than wasted bandwidth, but it does mean the rungs a
viewer sees can look uncorrelated in the logs.

## Tuning

The host passes these through `--ssts-algorithm-params`; the built-in default is
in brackets.

| key                   | default | meaning                                            |
| --------------------- | ------- | -------------------------------------------------- |
| `bnb.safe_factor`     | 0.2     | backlog / budget below which the set is healthy    |
| `bnb.danger_factor`   | 0.7     | backlog / budget above which a discard is imminent |
| `bnb.up_streak`       | 3       | healthy, non-growing groups before a climb         |
| `bnb.down_streak`     | 1       | dangerous groups before a drop (1 = immediate)     |
| `bnb.timeout_streak`  | 3       | groups of timeouts that force a drop               |
| `bnb.drain_window_ms` | 1500    | how long the rung holds after a change             |

The research branch this came from read the same five knobs from
`SSTS_BNB_SAFE`, `SSTS_BNB_DANGER`, `SSTS_BNB_UP_STREAK`,
`SSTS_BNB_DOWN_STREAK` and `SSTS_BNB_TO_STREAK`, from inside the algorithm. The
equivalent now is
`--ssts-algorithm-params bnb.safe_factor=…,bnb.danger_factor=…`, and the reason
for the move is in [`../../../README.md`](../../../README.md) under tuning.

## Identity

Wire id `0xff03` (`PRIVATE_ALGORITHM_ID_BASE + 2`). The research branch ran this
as algorithm `3`, which sits in the range the specification reserves for IETF
allocation; the private range is where an implementation's own experiments
belong, so a later standard allocation cannot collide with it.
