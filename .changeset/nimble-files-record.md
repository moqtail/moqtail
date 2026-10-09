---
'moqtail-ssts': patch
---

Give each WASD version its own file — `classic.rs`, `v2.rs`, `v3.rs`, each
declaring its `FACTORY` — with the shared engine left in `mod.rs` and
`state.rs`, so the comparison's three variants are visible one per file.

Document the measured shortcoming of the retroactive versions, accepted as
a property of the design: on `PROFILE_JITTER` a 500 ms dip makes v2 and v3
fall two rungs for a path that has recovered, because the drain the dip
leaves behind — its bytes in the shaper plus the repair bytes the downshift
ordered — overlaps the next group at the delivered tier and reads as a
second congestion. A cooldown (raised bar, ended on the repairs or on a
clean reading) and an age-filtered depth were both built, measured on the
profile, and dropped; the reasoning and the numbers are in the README's
"What it does badly" and the mental note. No behaviour change: the
algorithms are exactly the three the sweep runs.
