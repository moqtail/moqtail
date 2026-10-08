# WASD — within-tier ASD, decided inside the group

**WASD** is *Within-tier Active Stream Depth*: ASD's depth signal counted only
at the rung actually being delivered. It was named `eeasd` in the tree until
the rename. Three versions are advertised; they are one state machine and
differ only in how far a downshift reaches back into the groups it overtook.

The third of the compared algorithms: [`../asd_v11`](../asd_v11)'s depth
signal, measured only at the quality being delivered, read again while a group is
still in flight, with a downshift that reaches back into the groups it overtook
(the reach is what the versions tune).

It exists because the earlier algorithms share an assumption: that a group is
the unit of time. Groups are the unit the _media_ offers, but the path changes
faster than 25 groups a second, and by the time a group boundary arrives the
decision is already late. And they all read the connection's whole queue as its
congestion, which makes the recovery from a downshift look like the problem the
downshift was meant to fix.

## The versions

All three walk the same ladder from the same readings — same depth read at
the delivered tier, same mid-group monitor, same exploration and cautious
streaks, same `wasd.reads` knob — so the comparison measures exactly one
thing. On a downshift from 1080p to 720p at group 4, with groups 1–3 still in
flight at 1080p:

| Name | Wire id | What the downshift does with the groups it overtook |
| --- | --- | --- |
| `wasd` (classic) | `0xff04` | Nothing retroactive: group 4 goes out at 720p and groups 1–3 finish at 1080p. |
| `wasd-v2` | `0xff05` | Cancels the **newest** overtaken group — group 3 — and has it re-sent at 720p. Groups 1 and 2 finish where they are. |
| `wasd-v3` | `0xff06` | v2 plus one step back: group 3 re-sent at 720p, and the older groups (1, 2) cancelled and re-sent **two rungs below where they were** — 480p after a 1080p→720p drop, floored at the bottom rung. The older a group is, the less of its delivery window the re-send has left to finish inside. |

`wasd-v3` under id `0xff04` is what the tree ran before the versions were
split out; the staged behaviour now lives at `0xff06` and `0xff04` is the
classic.

The rationale for reaching back at all, what was settled on the way and what
is deliberately still open, are kept in [mental-note.md](mental-note.md),
along with where each piece of the mechanism lives.

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

(How far this goes is the version: the classic asks for nothing, v2 for the
newest overtaken group alone at the new rung, v3 for every one of them,
age-staged as below.)

The selection names the sets whose in-flight deliveries it passed (`supersede`),
as staged by age: the newest of those groups is re-sent at the member this
decision selected, and every older one one rung below it (the ladder's bottom
being the floor). This relay's mechanism answers by flipping each group to the
member the request laid out for it, so the objects that have not arrived yet
come from that member, resetting the streams still carrying the old one with
`Cancelled`, and marking the groups as re-sent. The reset stream tells the
subscriber that the tail is gone rather than merely late, which is the
difference between a subscriber that waits and one that moves on.

The staging exists because the clock, not the quality, is the constraint. An
older group started earlier, so less of its delivery window — the buffer its
subscriber negotiated — remains, and the re-send has to finish inside what is
left; one rung lower is how it finishes sooner. The newest group still has the
full window, so it keeps the rung the decision landed on. A host that does not
re-send at all may ignore the flag: it is a request about how a re-send is laid
out, not about whether one happens.

## Knobs

`wasd.reads` (default 2): how many consecutive mid-group readings the monitor
waits for. It is the only one, and it is the one worth moving, because it is a
statement about media pacing: on a 25 fps stream with 100 ms groups the default
filters a normal group tail, and on media whose groups are seconds long the
window it takes to see real congestion inside one is much longer. Setting it to
1 turns the monitor into a reaction to a single sample, which is not a monitor;
it is clamped to 1 rather than 0.

## What this is not, and where the rest of the experiment went

Two pieces of the research version live in the relay, not in this crate, because
this crate has no mechanism in it: no streams, no cache, no clock of its own.
They are described here so that what the algorithm's decisions _mean_ on this
relay is legible from the algorithm's directory.

**The replay is in the relay's forwarding path.** A group this algorithm
overtook is re-sent in full: the relay notices that the first object it forwards
for that group is not the subgroup's first, opens a new stream on the member the
re-send runs on — the decision's own rung for the newest overtaken group, one
rung below for each older one — puts the cached head (objects `0..N-1` of that
subgroup) on it from the track cache, and lets the live objects continue behind.
Nothing is
waited for: a head that is not in the cache — evicted, or a member whose copy of
the group started later — means no re-send, and the switch stands on its own. The
old delivery is reset rather than reused, and the new stream carries the
publisher's own header for that subgroup, which is what makes `FIRST_OBJECT`
correct: a re-send starts where a subgroup starts. See
`apps/relay/src/server/subscription.rs`.

**Best effort, by decision.** A re-send costs the bytes of a group that was
already partly sent, on a connection that is congested by definition, and it
delivers media the subscriber may already have given up on. It is bounded by the
deadline above and by the cache, and by nothing else: there is no cutoff on how
much of a group has already been sent, and no estimate consulted before
committing the bytes. On a link that is congested rather than merely busy, that
shows up as a group that arrives twice and still gets discarded.

**The extension's `umin` and `rho` slots.** The research version read a third
extension slot as a timeout and stored two others unused. Nothing here reads
`algorithm_params`: the timeout they carried is now `subscriber_buffer_ms`, which
is the field with mechanism meaning, and the two unused slots were dropped rather
than reserved. An algorithm that needs a knob can take one; the subscriber sends
them in the order the algorithm defines.

## What it does badly

**It has no cooldown, which is a claim, not an omission.** wasd says a
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
[`../experimental/bnb`](../experimental/bnb) exists to break; wasd kept it because the research version
compared the depth _signal_, and a per-set index would have changed two things at
once.
