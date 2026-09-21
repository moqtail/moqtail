# eeasd — depth at the tier, decided inside the group

The third experimental algorithm: [`../enhanced_bp`](../enhanced_bp)'s depth
signal, measured only at the quality being delivered, read again while a group is
still in flight, with a downshift that reaches back into the groups it overtook.

It exists because the other three share an assumption: that a group is the unit
of time. Groups are the unit the _media_ offers, but the path changes faster than
25 groups a second, and by the time a group boundary arrives the decision is
already late. And they all read the connection's whole queue as its congestion,
which makes the recovery from a downshift look like the problem the downshift was
meant to fix.

## What it measures

The depth is the number of live forwarding streams carrying, for each active set,
the member at the tier the last decision chose. Nothing else counts:

- streams at other rungs of the same set belong to a quality that has already
  been rejected;
- streams opened to re-send a group the relay overtook are the relay's own doing.

Both exclusions are load-bearing. Count the left-behind rungs and a set that is
recovering looks congested; count the re-sends and every downshift produces the
evidence for the next one, in a loop the operator watches as a ladder walking
itself to the bottom rung on a healthy link.

## When it decides

At every group boundary, as the others do — and also on the host's periodic
sample of the group in progress. A single deep mid-group reading is ignored:
the tail of a perfectly healthy group produces exactly one, because the next
group's stream opens before the previous one has drained. Two in a row, at the
downshift depth or above, step down one rung without waiting for the boundary.
The streak re-arms when it fires, and the dip costs a full fresh clean streak
before the climb resumes.

## What a downshift does to the groups it overtook

The selection names the sets whose in-flight deliveries it passed (`supersede`).
This relay's mechanism answers by flipping those groups to the new member, so the
objects that have not arrived yet come from it, resetting the streams still
carrying the old one with `Cancelled`, and marking the groups as re-sent. The
reset stream tells the subscriber that the tail is gone rather than merely late,
which is the difference between a subscriber that waits and one that moves on.

## Knobs

`eeasd.reads` (default 2): how many consecutive mid-group readings the monitor
waits for. It is the only one, and it is the one worth moving, because it is a
statement about media pacing: on a 25 fps stream with 100 ms groups the default
filters a normal group tail, and on media whose groups are seconds long the
window it takes to see real congestion inside one is much longer. Setting it to
1 turns the monitor into a reaction to a single sample, which is not a monitor;
it is clamped to 1 rather than 0.

## What this is not, and where the rest of the experiment went

Two pieces of the research version live in the relay, not the algorithm, and are
**not implemented here**. Both were left out deliberately rather than
half-ported, and the algorithm's behaviour differs accordingly.

**The full-group re-send from the cache.** The research version replayed an
overtaken group from its first object at the new quality, so the subscriber got a
decodable group instead of a tail. Here the overtaken delivery is cancelled and
the new member's own objects resume from wherever that group had got to, which
means the subscriber can receive a group that starts mid-way and may not decode
until the next one. That is a real cost, and it is the reason this algorithm is
opt-in on a private id and not a default.

The replay belongs in `apps/relay/src/server/subscription.rs`, in the branch that
already opens a stream from a cached subgroup header for a subscriber that joined
mid-subgroup — the mechanism exists, and what is missing is a trigger keyed on the
group being a re-send. It has consequences that are not this algorithm's to
settle: the `FIRST_OBJECT` flag on a stream that does not start at the subgroup's
first object (already a documented conformance gap on the cache-join path), and
what a replayed object does to the per-subscription "last delivered location"
that STATUS reporting reads.

**The halved discard timeout is implemented.** A group this algorithm causes to
be re-sent gets half the buffer its subscriber negotiated for the set, carried in
the request (`Selection::supersede_with_deadline`) and applied by the host to the
streams of that group alone. It is part of what selecting this algorithm means:
the relay is finding out whether the lighter delivery makes a deadline, not
whether the heavy one did. A set that negotiated no buffer has granted no
permission to discard, and gets no deadline — `None`, which leaves the relay's
rule of never discarding in charge. Half of 1 ms would be 0, and 0 means never, so
the floor is 1.

**The extension's `umin` and `rho` slots.** The research version read a third
extension slot as a timeout and stored two others unused. Nothing here reads
`algorithm_params`: the timeout they carried is now `subscriber_buffer_ms`, which
is the field with mechanism meaning, and the two unused slots were dropped rather
than reserved. An algorithm that needs a knob can take one; the subscriber sends
them in the order the algorithm defines.

## What it does badly

**It has no cooldown, which is a claim, not an omission.** eeasd says a
downshift drains its own signal, because the streams that were congested are no
longer at the tier being measured. That is true when the downshift works. When it
does not — when the depth comes from a subscriber that is simply slow, or from a
path that cannot carry even the bottom rung — there is nothing to stop the
ladder, and the only thing that halts it is the bottom.

**The mid-group monitor is a second decision path, and the two can argue.** A
boundary reading and a mid-group reading of the same group are taken at different
distances into it and can disagree about the tier. The state machine resolves that
by preferring whichever came last, which is a policy rather than a truth.

**One tier for the whole connection, still.** Every set the algorithm owns moves
together, so a connection carrying a 4K ladder and an audio ladder degrades both
to hear the audio better. This is the assumption that
[`../bnb`](../bnb) exists to break; eeasd kept it because the research version
compared the depth _signal_, and a per-set index would have changed two things at
once.
