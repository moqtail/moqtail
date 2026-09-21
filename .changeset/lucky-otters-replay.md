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

A re-sent group gets half the discard deadline its subscriber negotiated for the
set, which is part of what selecting the algorithm means; a set that negotiated
no buffer gets no deadline, because there is no permission there to shorten.

The one piece of the research version still absent is the replay of an overtaken
group's cached head, named in the algorithm's README with what is missing and
where it would go: without it a switched group can reach the subscriber starting
mid-way.
