# ASD v1.1 — backpressure with slow start

The depth signal of [`../asd`](../asd) (Active Stream Depth), reached faster at the
start of a connection.

A connection begins on the lowest rung and, while it is exploring, climbs one
rung per uncongested group instead of per five. Exploration ends — permanently —
at the first of:

- depth reaching 2, the congestion threshold;
- the tier arriving at the top of the longest ladder among the sets that have a
  subscriber.

After that the two algorithms behave identically: five consecutive clean groups
climb one rung, depth 2 drops one and starts the settle-down period.

## Why this is a separate algorithm and not a change to the other one

The exploration trades the thing slow start always trades: it finds the top of
the ladder in a third of the time, and it overruns the path harder while
looking. With a depth-based signal the evidence of overrunning is a stream the
subscriber could not drain, which for real media means discarded objects on a
group the subscriber is waiting for. On a link with a short group period that
cost is small and paid once; on a link where a group is several seconds of video
it is not. Whether the trade is worth making is a property of the deployment, so
both algorithms are offered and the operator picks per relay.

## What it does badly

Everything the base algorithm does badly, which is documented in
[`../asd/README.md`](../asd/README.md) and not repeated there
by reference: summed depth across sets, timeouts counted as queue depth, one
index for all of a connection's sets.

Two of its own:

**The exploration ends once, and a connection that loses its path early never
gets it back.** A subscriber that starts on a congested link spends its whole
session climbing at five groups per rung, even after the link improves by an
order of magnitude. That is a deliberate consequence of the rule — the
connection has evidence the path failed, so caution is justified — but it is
wrong for the common case of a connection that started behind someone else's
download and then had the link to itself.

**Congestion during the exploration is measured in the same coarse units as
everything else.** One group of depth 2 ends the exploration for the lifetime of
the connection. A single stalled subgroup — a keyframe larger than the pacing
window allows for — is enough to end it. Making the end-of-exploration condition
"two congested groups" instead of one would cost the property that makes slow
start safe, which is that it stops the moment the path says stop.

**Reaching the top is not the same as the path being exhausted.** The ladder's
tallest set defines where exploring stops, so a connection serving a 1080p ladder
stops exploring at 1080p even with capacity to spare. That is correct as far as
it goes — there is nothing above the top rung to serve — but it means the
algorithm cannot report "the path is much better than anything I can send",
which is the measurement an operator wants when deciding whether to publish more
rungs.

## State

Per connection, ASD's `TierState` plus one flag: whether the connection
is still exploring. The flag is one-way.

## Identity

Name: **ASD v1.1**, wire id `0xff02` (`PRIVATE_ALGORITHM_ID_BASE + 1`); `2` on
the research branch, where it was called *enhanced backpressure* (tree name
`enhanced_bp`, since this directory's `asd_v11`).
See [`../asd/README.md`](../asd/README.md) for the mapping.
