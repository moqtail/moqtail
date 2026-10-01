---
'relay': patch
'moqtail-ssts': patch
---

A group is decided once. Every member of a switching set asks the ABR for
a decision when the group's first Object reaches its gate, and every ask
used to run a full group-boundary cycle: a stateful algorithm walked its
state once per ask instead of once per group, and the answer for a group
could change while that group's Objects were passing the gate. The head
was then dropped on the old side of the flip and the group reached the
subscriber without its head, which no decoder can take. Now the first
boundary decides and later asks only wake the waiters; a group whose
decision has been pruned behind the live edge decides again, so an
Object arriving late is never left waiting for one.
