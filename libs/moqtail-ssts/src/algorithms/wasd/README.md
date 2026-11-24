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

The re-send exclusion is only half the rule: a re-sent stream does not count as
depth, but it does veto a clean boundary. While a repair is in flight the
delivery it replaced has not resolved, so the boundary cannot buy a climb — the
clean streak restarts once the repair is over. Without the veto the ladder
climbs back onto the rung the repair came from: on `PROFILE_LANCSD1` it went
900p -> 1080p at t=31.99 s, 40 ms before the same streams pushed it back down.

The climb is also held while the rung above has an open stream of its own. That
stream is not depth at the current tier and never causes a downshift, but it is
not a clear target either: the new group's stream would be the second at that
rung and the monitor would read the climb as congestion. Classic, which never
resets the streams a downshift abandoned, hit this on every climb back to
1080p (three old 1080p streams, reverted in 27-51 ms).

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
a climb waits for is priced by the step itself: the relative bitrate jump
divided by the subscriber's risk tolerance, with an optional floor and cap
(`wasd.rho`, `wasd.umin` and `wasd.umax`, below).

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
the monitor waits for. Two is the measured default on this testbed: on the
same runs, a third reading bought almost no throughput, quality or stability
back and cost latency and frozen time, because the sample clock is the objects
sent and the wait grows exactly when the path is slow. It is worth moving when
the media paces differently than the sample the default was tuned on: on a
25 fps stream with 100 ms groups the default filters a normal group tail, and
on media whose groups are seconds long the window it takes to see real
congestion inside one is much longer. Setting it to 1 turns the monitor into
a reaction to a single sample, which is not a monitor; it is clamped to 1
rather than 0.

`wasd.rho` (default 0.25, in `(0.01, 1]`): the probing risk tolerance, and
the mechanism the climb is priced by. The steady climb waits

```text
U(k) = min( u_max, max( 2, u_min, ceil( ((R_{k+1} - R_k) / R_k) / rho ) ) )
```

clean groups. The step term prices the jump itself: a 3x rung is a 3x bigger
bet than a 1.2x one, so the observation window scales with it. Lower is more
cautious. 0.25 is the draft's default: on the 0.60/1.18/2.89/5.75 Mbps ladder
it asks 5, 4 and 3 clean groups. A zero nominal rate counts as a 100 % step,
and the top rung is the draft's fixed two.

`wasd.umin` (default 2): a floor under the priced wait, in clean groups. The
draft fixes the minimum at two, so a lower value — including zero — is raised
to two rather than removing the floor. Setting it higher takes direct control
and lets `rho` stop deciding the minimum.

`wasd.umax` (default unset): a cap on the wait, in clean groups; `0` or an
unset key is no cap. It exists to bound recovery time when `rho` is low: at
`rho = 0.05` a steep ladder asks for tens of clean groups, which on a short
trace means never climbing again.

Both are rails, not mechanisms, and they clamp the same number: **setting
`wasd.umin` equal to `wasd.umax` pins `U(k)` to that constant and makes `rho`
inert** — that is the "I do not want rho" setting, with no separate switch.
A cap below the floor is raised to the floor, so the floor is never undercut.

There is no derived floor. It used to default to one buffer-depth,
`ceil(B / G)`, computed from the subscriber's buffer and the observed group
cadence; that made the buffer the primary control through a formula, and any
such derivation is an arbitrary number wearing a costume. `ceil(B / G)`
remains a good *recommended* value for an operator to set — `wasd.umin=4` for
a 1.6 s buffer with 500 ms groups — but it is now chosen, not assumed.

**Subscriber signalling.** The set's algorithm parameters carry the same
numbers per connection, in the draft's fixed extension order:
`[rho_pct, U_min, U_max]`, after the latency budget. A signalled value
overrides the operator's for that subscriber's sets. `rho_pct` is `1..=100`,
with `0` meaning the publisher default (25 %) and values above 100 clamped to
100 %. `U_min` is a count of media groups, raised to the draft floor of two;
`0` also means the publisher default. `U_max` is the third field: non-zero
caps the wait, and `0` means unbounded rather than falling back to the
operator's cap.

## What this is not, and where the rest of the experiment went

Two pieces of the research version live in the relay, not in this crate, because
this crate has no mechanism in it: no streams, no cache, no clock of its own.
They are described here so that what the algorithm's decisions _mean_ on this
relay is legible from the algorithm's directory.

**The replay is in the relay's forwarding path.** A group this algorithm
overtook is re-sent from the member the decision chose: the relay opens a new
stream on that member, puts the cached run (from the subgroup's first object up
to the live object, or the whole cached subgroup when no live object is coming)
on it, and lets the live objects continue behind. The cache-only arm is what
covers a Group already complete at the relay when a downshift overtakes it: no
further Object will arrive to trigger the live path, so the replacement is
armed at supersede time. A Group whose cached Objects were all already written
is not re-sent. The old delivery is reset rather than reused. The new stream
carries the publisher's own header when it is still open; if the publisher's
stream has closed, the relay synthesizes a first-object header for the cached
run, which is why that run must start at the subgroup's first Object. A cache
that has evicted the Group means no re-send, and the switch stands on its own.
See `apps/relay/src/server/subscription.rs`.

**Best effort, by decision.** A re-send costs the bytes of a group that was
already partly sent, on a connection that is congested by definition, and it
delivers media the subscriber may already have given up on. It is bounded by the
deadline above and by the cache, and by nothing else: there is no cutoff on how
much of a group has already been sent, and no estimate consulted before
committing the bytes. On a link that is congested rather than merely busy, that
shows up as a group that arrives twice and still gets discarded.

**The extension's `rho`, `U_min` and `U_max` fields.** The research version
read a third extension slot as a timeout and stored two others unused. The
timeout is now `subscriber_buffer_ms`, and the draft's three adaptation fields
follow it in the fixed order `[rho_pct, U_min, U_max]`. A subscriber signals
its probing risk tolerance as a percentage (`1..=100`; `0` default, `>100`
clamped), its minimum upshift stability threshold as a group count (raised to
the draft floor of two), and its cap (`0` unbounded). The operator's
`wasd.umin` / `wasd.umax` apply only when the subscriber did not make the
field a decision.

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
Measured later, and worth knowing before blaming the algorithm: the group
delivery order the subscription negotiates amplifies this. Under the MOQT
default (Ascending) an abandoned older group outranks the newly selected tier,
so the new tier's stream starves and the second drop follows; requesting
requesting Descending group order removed it on `PROFILE_JITTER` (3->2 at 10.5 s then 2->3 at
11.7 s, instead of 3->2 then 2->1). See `ssts-docker/README.md`.

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

**One tier for the whole connection.** Every set the algorithm owns moves
together, so a connection carrying a 4K ladder and an audio ladder degrades both
to hear the audio better. The relay therefore accepts only one switching set
while a version of this algorithm is in use; multi-set support needs a per-set
ladder first.
