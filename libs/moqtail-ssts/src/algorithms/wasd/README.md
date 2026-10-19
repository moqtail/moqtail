# WASD — within-tier ASD, decided inside the group

**WASD** is *Within-tier Active Stream Depth*: ASD's depth signal counted only
at the rung actually being delivered. It was named `eeasd` in the tree until
the rename. Three versions are advertised; they are one state machine and
differ only in how far a downshift reaches back into the groups it overtook.
Each version is declared, with its own documentation, in its own file:
[`classic.rs`](classic.rs), [`v2.rs`](v2.rs), [`v3.rs`](v3.rs).

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

On every sample the host takes — the group boundary and the periodic sample of
the group in progress alike — through one state machine. What the two samples
may conclude differs by what they can see.

The boundary's reading is certain, and acts alone: when a group's first Object
arrives and two streams of the delivered tier are still open, the delivery is
two groups behind — the new group's stream has not opened yet, so the handoff
overlap cannot fake it. One such reading steps down one rung and ends the
exploration.

The mid-group reading is not certain, and waits: a single deep sample is the
normal handoff overlap — the new group's stream open while the previous one's
FIN-ACK is still in flight — so the monitor acts only on two consecutive
congested samples (`wasd.reads`). A reading that does not fire touches
nothing, least of all the clean streak the climb counts on; a downshift that
does fire costs a full fresh clean streak before the climb resumes, and the
streak re-arms so one burst cannot walk the ladder tick by tick.

Upshifts belong to the boundary alone — the moment the forward gate is waiting
to record that group's track — so the tier never moves mid-group, and
`current_index` never disagrees with what is on the wire. The cautious streak
a climb waits for is the subscriber's own, not a constant: at least one
buffer-depth of clean groups, and more across ladder rungs whose bitrates are
far apart (`wasd.rho`, below).

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
out, not about whether one happens. A repair is not immune to the next decision
either: a second step down overtakes a re-send still in flight above the new
rung, cancels it and re-sends it at the lower one, and the repair's own elapsed
time — measured from when the repair started, not when the group did — donates
the second re-send's deadline.

## Knobs

`wasd.reads` (default 2): how many consecutive congested mid-group readings
the monitor waits for. It is worth moving when the media paces differently
than the sample the default was tuned on: on a 25 fps stream with 100 ms
groups the default filters a normal group tail, and on media whose groups
are seconds long the window it takes to see real congestion inside one is
much longer. Setting it to 1 turns the monitor into a reaction to a single
sample, which is not a monitor; it is clamped to 1 rather than 0.

`wasd.rho` (default 1.0, in `(0, 1]`): the probing risk tolerance. The
steady climb waits

```text
U(k) = max( U_min,  ceil( ((R_{k+1} - R_k) / R_k) / rho ) )
```

clean groups: at least `U_min` of them — one buffer-depth, `ceil(B / G)`
from `B` the subscriber's negotiated buffer and `G` the observed group
cadence, unless a signalled or operator floor replaces it — and more across
rungs that are far apart in bitrate, because a 3x jump is a 3x bigger bet
than a 1.2x one. Lower is more cautious; 1.0 climbs on the floor alone
whenever the ladder's relative steps are smaller than one buffer-depth.

`wasd.umin` (default unset): an operator's floor for `U_min`, in clean
groups, used in place of the derived `ceil(B / G)` when set. Unset, the
floor is that recommended buffer-drainage horizon, and the written five
still answers when neither clock is known.

**Subscriber signalling.** The set's algorithm parameters carry the same
two numbers per connection, and signalled values override the operator's
for that subscriber's sets. With one parameter it is the tolerance: a
percentage (`1..=100`) for `rho`. With two or more, slot 0 is `U_min` — a
count of media groups, `0` meaning "not signalled" — and the trailing
parameter is the `rho` percentage. An unsignalled (or zero) `U_min` leaves
the derived `ceil(B / G)` in place, unless the operator set `wasd.umin`:
the floor arrives with the connection instead of being signalled.

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
extension slot as a timeout and stored two others unused. The timeout is now
`subscriber_buffer_ms`, which is the field with mechanism meaning; the two
dormant slots are live here. A subscriber signals its probing risk tolerance
`rho` as a percentage (`1..=100`) in the trailing `algorithm_params` slot, and
its minimum upshift stability threshold `U_min` — a count of media groups — in
slot 0 of the two-slot layout; a zero, or a one-slot vec, means unsignalled,
and the floor then derives itself from the subscriber's own buffer and
observed cadence, `ceil(B / G)`. The subscriber sends them in the order the
algorithm defines.

## What it does badly

**The retroactive versions fall two rungs for a dip that lasts one: a
measured shortcoming, accepted.** On `PROFILE_JITTER` (a 500 ms dip to
2 Mbps) v2 and v3 correctly fall one rung, then fall a second one about a
second later — for a path that has already recovered. The mechanism is the
drain: the dip leaves its bytes in the shaper, the downshift's own re-send
bytes join that queue, the next group's stream stays open into the one
after it, and two live streams at the delivered tier — the shape of a
drain — read as a second congestion to the depth. The classic does not
cascade, because it neither spends repair bytes nor resets the deliveries
that were draining. Two families of fix were built and measured on the
same profile and both were dropped: raising the downshift bar for a while
(a cooldown, ended either when the re-sends concluded or when the tier next
read at or below target) failed because the overlap is *preceded by clean
readings* — any cooldown that ends on evidence ends before the overlap
starts; counting only streams older than a group period (an age-filtered
depth) suppressed the cascade but charged a group of seeing to genuine
congestion, delaying the ladder's real responses. The comparison therefore
ships the shortcoming visible: reaching back costs an overreaction on
short dips, and v2/v3 are measured against the classic with it in place.

**A downshift that does not work has nothing to stop it.** The depth at the
delivered tier drains when the downshift works; when it does not — when the
depth comes from a subscriber that is simply slow, or from a path that
cannot carry even the bottom rung — there is nothing to stop the ladder,
and the only thing that halts it is the bottom.

**A downshift can act mid-group; an upshift never does.** The asymmetry is
deliberate and it belongs to the relay's gate, not to the algorithm's taste: a
mid-group upshift would move the tier for a group whose member the gate has
already recorded, so it could only ever take effect at the next group anyway.
The state machine therefore evaluates upshifts only on the first sample of a
new group, which keeps the tier in sync with the wire — at the price of a
recovered path waiting, on average, half a group longer than it strictly has
to.

**One tier for the whole connection, still.** Every set the algorithm owns moves
together, so a connection carrying a 4K ladder and an audio ladder degrades both
to hear the audio better. This is the assumption that
[`../experimental/bnb`](../experimental/bnb) exists to break; wasd kept it because the research version
compared the depth _signal_, and a per-set index would have changed two things at
once.
