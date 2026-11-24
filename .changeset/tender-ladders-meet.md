---
'moqtail-ssts': minor
'relay': patch
---

Add the experimental tier controllers: `asd_v11` — ASD v1.1, landed as
`enhanced_bp` and renamed at the naming pass (0xff02) — which explores
the quality ladder one rung per clean group until the top or the first
congestion, then behaves like the ASD selector.

Both are opt-in on private ids, like the ASD selector before them:
`--ssts-algorithms 0,0xff02` to run them. The ids, not the algorithms, are
negotiated, so a subscriber that asks for one this relay does not run is answered
with the ids it does.

The relay now also feeds them what the contract had started to withhold: a
monotonic clock, the reason a decision is being asked for (a new group, a sample
inside one, or a stream the transport gave up on), the live forwarding streams
with their ages, which set lost how many streams since the last decision, and the
operator's `--ssts-algorithm-params key=value` pairs.
