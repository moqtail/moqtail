---
'moqtail': minor
---

`moqtail/player` exports the `SSTS` extension and `SstsSource`. With `extensions: [SSTS]` the player offers SSTS algorithms 0 and 0xff01 at setup, and a switching set source subscribes every track into one switching set (`SwitchingSetAssignment`, `activate` equal to the set size, no FORWARD) and forwards whichever track the relay streams. `selectRenditions()` adds and removes tracks, `pauseSwitching()` and `resumeSwitching()` change `activate`, and `setAlgorithm()` moves the set to another negotiated algorithm.
