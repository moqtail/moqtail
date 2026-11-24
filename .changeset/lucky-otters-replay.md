---
'moqtail-ssts': minor
'relay': patch
---

Add `backpressure` (0xff04, landed as `eeasd` and renamed at the naming pass), the
third experimental tier controller: the depth measured
only at the quality actually being delivered, read again while a group is still
in flight, with a downshift that reaches back into the groups it overtook — their
deliveries are cancelled and their remaining objects resume on the new member.

This is the first decision an algorithm can make that destroys work in progress,
so `Selection` now says two things rather than one: which member each set
forwards, and which sets' in-flight deliveries the decision overtook. The second
is a request. Algorithms still cannot reset a stream, reach a cache or re-send an
object; the host implements the request as it sees fit, and this relay cancels.

A re-sent group gets half the discard deadline its subscriber negotiated for the
set, which is part of what selecting the algorithm means; a set that negotiated
no buffer gets no deadline, because there is no permission there to shorten.

A group that is re-sent is re-sent in full: the relay opens a new stream on the
member the decision landed on, replays the group's cached head onto it - the
publisher's own subgroup header, so the stream starts where a subgroup starts -
and lets the live objects continue behind. A head that is not in the cache means
no re-send rather than a wait: the switch itself stands on the objects that
follow.
