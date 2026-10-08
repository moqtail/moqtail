---
'moqtail-ssts': minor
'relay': patch
---

Name the experimental algorithms for the comparison they belong to.

- `backpressure` is now **ASD** (Active Stream Depth), the professor's name
  for the signal: module `algorithms::asd`, name string `asd`, id `0xff01`.
- `enhanced_bp` is now **ASD v1.1**, a small increment on the same signal:
  module `algorithms::asd_v11`, name string `asd-v1.1`, id `0xff02`.
- `eeasd` is now **WASD** (Within-tier Active Stream Depth): it counts only
  the streams at the tier being delivered. Module `algorithms::wasd`, name
  string `wasd`, id `0xff04`. Its tuning key is `wasd.reads` (formerly
  `eeasd.reads`).
- `bnb` (0xff03) is parked, not renamed: still compiled and tested, but no
  longer registered, so it cannot be advertised or negotiated and its id is
  left unused. It moved to `algorithms::experimental::bnb`.

The ids on the wire are unchanged; only the 0xff03 one stops being offered.
Names are cosmetic on the wire (negotiation is by id), but the relay's log
lines now read `SSTS asd:`, `SSTS asd-v1.1:` and `SSTS wasd:`.
