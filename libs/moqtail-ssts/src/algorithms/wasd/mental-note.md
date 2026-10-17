# Mental note — what was decided, and what is still mine to decide

Notes to self about the retroactive downshift: the choices that were made on
purpose, the ones that were made because they matched what the media actually
does, and the ones that were left open on purpose. Written so that a future me,
six weeks and one bad capture from now, does not "fix" a decision by
re-deriving it from scratch — or mistake an open question for an oversight.

## Decided, and why

**The re-send inherits the remaining buffer of the stream it replaces.** The
old rule was half the negotiated buffer — a guess at how much of the window a
re-send could still make. The measurement was available the whole time: the
overtaken stream carries `age_ms`, so `subscriber_buffer_ms - age_ms` is exactly
what the subscriber's window has left, and the replacement stream has precisely
that much left too. The newest overtaken group donates, because it is the group
the re-send replaces in the delivery order; v3's staging already prices the
older groups one rung lower for having less. No buffer, no deadline: a set that
negotiated nothing granted no permission to discard, and `None` leaves the
relay's never-discard rule in charge. The 1 ms floor is an edge, not a policy:
an overtaken stream already past the buffer leaves a re-send expired on
arrival, and 0 in this contract means *never*, which is the opposite.

**No buffer, no deadline.** A set that negotiated nothing granted no permission to
discard its media, and there is nothing to halve. The request carries no deadline
and the relay's rule of never discarding stays in charge. The floor of 1 ms is an
edge, not a policy: `0` means _never_ in this contract, so halving a 1 ms buffer
must not fall through the arithmetic into an infinite deadline.

**The deadline applies to the re-sent group's streams and to nothing else.** Not
to the delivery that was overtaken, not to the set's other groups.

**Cache miss means no re-send, and that is the whole fallback.** Nothing waits on
the publisher. If the head is gone — evicted, or this member's copy of the group
started later — the relay cancels the old delivery and switches on the objects
that follow. Losing the chance to re-deliver media that was already in the building
is not a reason to stall a live stream.

**No synthesized subgroup headers.** The replay needs the publisher's own header
for that subgroup, and without it there is no replay. A synthesized header would
describe a subgroup whose first object is not the first object being sent, which is
the one thing the wire format has no answer for; the cache-join path already
carries that gap knowingly and this path is not going to add a second one. "We
could not re-send" is a visible, honest outcome.

**Re-send on a new stream, never the reset one.** The overtaken stream was reset;
a stream that was reset does not become a new delivery.

**Per subgroup, not per group.** The subscriber reassembles subgroups. A sibling
subgroup's head would be written to a stream the live object never joins — media
nobody asked for, on a connection that is congested by definition.

**No cutoff on how much of the group was already sent.** This was measured, not
assumed: the groups that get re-sent are always less than half sent, so a cutoff
would filter nothing and would only hide the cases where the assumption stops
holding. If it stops holding (much longer GOPs, much larger groups), the fix is a
cutoff rule — see the open list.

**Bandwidth duplication is accepted.** A re-send costs a group's bytes twice, on a
congested link, and may still be discarded. A client that wants ultra-low latency
gets half-re-sent groups and gaps. That is the nature of streaming, and the
alternative — never switching mid-group — is the thing this algorithm exists to
avoid.

**Location bookkeeping is left alone.** Both trackers are max updaters, so
replaying older objects cannot drag the position the switch context reads
backwards. A re-send is the one case where media deliberately travels to a point
the subscriber has been past, and the new stream's own last-object bookkeeping
starts empty rather than inheriting the overtaken stream's, so the object-id deltas
on the wire stay correct.

**The replay is triggered by the re-send mark, not by "object > 0 and no
stream".** Ordinary mid-group joins must not get a replay they did not ask for. The
mark is set only when a decision overtook the group.

**The short-dip double downshift is accepted, not patched.** Measured on
`PROFILE_JITTER`: a 500 ms dip makes v2 and v3 fall one rung correctly and then a
second one ~1 s later, because the drain — the dip's bytes in the shaper plus the
repair bytes the downshift ordered — keeps the next group's stream open into the
one after it, and the overlap reads as congestion at the delivered tier. Two fixes
were built, tested on the profile, and removed. (1) A cooldown: raise the
downshift bar by one stream after a downshift; ended when the re-sends concluded
— the drain outlives the repairs by a reading or two — or ended at the first clean
reading — the overlap is *preceded* by clean readings (the supersede resets the
tier to empty, then one normal group streams at target), so any evidence-based
end closes before the overlap starts. Both variants: 3/3 double drops, same as
no cooldown. (2) An age-filtered depth: count only streams at least one group
period old; it suppressed the cascade (0/3 double drops, the dips cost no tier
change at all, zero deadline misses) but charged a group of seeing — and, for
shallow congestion, the time for a queue to grow — to every genuine response.
The comparison is stronger shipping the shortcoming visible: reaching back costs
an overreaction on short dips, and that is a property of the design, not a knob
setting. The numbers are in the README's "What it does badly".

## Still mine to decide

1. **The cutoff**, if the "always less than half sent" observation stops holding —
   a group that is 95 % delivered should not be re-sent whole. Decide the rule
   (fraction of the group? remaining bytes vs. the remaining deadline?) when the
   media changes, not before.
2. **Streams parked in `finish()` when the downshift fires.** They are not cancelled
   by the reset — they finish gracefully, and since their group is now marked as a
   re-send, they close under the _inherited_ deadline
   (`subscriber_buffer_ms - age_ms`). That is defensible and it is
   what happens today. The research branch had a cancel-notify path for them. Decide
   whether the tail they finish sending is worth the machinery, once it shows up in
   a capture.
3. **Three knobs: `wasd.reads`, `wasd.rho` and `wasd.umin`.** Whether the inherited deadline
   is worth exposing (it should probably stay fixed — it is semantics, not
   tuning), and whether the monitor's depth thresholds deserve more knobs
   before someone asks for them.
4. **The `umin` / `rho` extension slots.** Both are live. `rho` is read from a
   trailing percentage in `algorithm_params`, overriding the operator's for
   that subscriber's sets. `umin` is signalled the same way — slot 0 of the
   two-slot layout, a count of media groups — and a zero or an unsignalled
   slot keeps the automatic default — the operator's `wasd.umin`, else
   `ceil(B / G)` from the subscriber's own buffer and observed cadence — so
   the floor arrives with the connection instead of being signalled whenever
   the client does not insist. The positional convention (one slot = rho
   alone; two slots = umin, rho) is the open surface: a float on the wire
   would need a different encoding, and the vec is `u64`.
5. **No end-to-end test of the replay.** The head selection is unit-tested
   (`cached_head`); the write path — new stream, header, head objects, live objects
   continuing behind — needs a two-hop harness that does not exist yet. Decide
   whether to build it before this is offered anywhere as more than an experiment.
6. **Re-sent bytes and reporting.** They count toward the PUBLISH_DONE stream count
   (they are real streams) and toward nothing that distinguishes them from fresh
   media. If REPORTING/STATUS ever needs to explain a duplicate group to a
   subscriber, that marker has to be invented, and it belongs to the mechanism.
7. **Multi-subgroup media for real.** The per-subgroup rule is right in principle; it
   has only been exercised with one subgroup per group, which is what the encoder
   setup produces. If a publisher ever uses several, watch what happens when only
   one sibling was in flight at the moment of the downshift.

## Where the pieces live

| Behaviour                                           | Place                                                       |
| --------------------------------------------------- | ----------------------------------------------------------- |
| "this group was overtaken" mark, its deadline       | `SwitchingSetManager::resent_groups`, `mark_group_resent`   |
| which streams the downshift overtook (ladder order) | `SwitchingSetManager::streams_above`                        |
| cancel, flip the decision, mark re-sent             | `ssts::controller::supersede_overtaken`                     |
| the deadline on close                               | `MOQTClient::close_stream`, `SstsState::resent_deadline_ms` |
| the replay itself                                   | `Subscription::replay_cached_head`, `cached_head`           |
| the head's streams excluded from depth              | `StreamSnapshot::resent`, `AbrInput::open_streams_at_track` |
| halving, and "no buffer, no deadline"               | `algorithms::wasd::resend_deadline_ms`                     |
