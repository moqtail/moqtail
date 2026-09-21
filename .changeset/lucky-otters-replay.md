---
'moqtail-ssts': minor
'relay': patch
---

Add `eeasd` (0xff04), the third experimental tier controller: the depth measured
only at the quality actually being delivered, read again while a group is still
in flight, with a downshift that reaches back into the groups it overtook — their
deliveries are cancelled and their remaining objects resume on the new member.

This is the first decision an algorithm can make that destroys work in progress,
so `Selection` now says two things rather than one: which member each set
forwards, and which sets' in-flight deliveries the decision overtook. The second
is a request. Algorithms still cannot reset a stream, reach a cache or re-send an
object; the host implements the request as it sees fit, and this relay cancels.

Two pieces of the research version are deliberately absent, and both are named in
the algorithm's README with what is missing and where it would go: the replay of
an overtaken group's cached head, without which a switched group can reach the
subscriber starting mid-way, and the halved discard timeout for re-sent groups,
which would spend a permission the subscriber negotiated for the set and not for
one group.
