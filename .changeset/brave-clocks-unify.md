---
'moqtail-ssts': minor
---

BACKPRESSURE's downshift and upshift paths become one. Every depth sample — group
boundary or mid-group — now feeds a single persistence counter, and a
downshift fires wherever the streak fills: the boundary has no monopoly on
acting, and no veto. It also ends the exploration when it fires, replacing
the old rule that only a boundary reading could. Upshifts are the
asymmetric half: evaluated only on the first sample of a new group — the
moment the relay's forward gate is waiting to record that group's track
decision — so the tier never moves mid-group and never disagrees with what
is on the wire. Consequence to know: a boundary reading that was alone in
its congestion no longer downshifts on its own; it needs one more congested
sample, from the monitor or the next boundary.
