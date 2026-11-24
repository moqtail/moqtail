---
'moqtail-ssts': minor
'relay': patch
---

Name the experimental algorithms for the comparison they belong to.

- `backpressure` is now **ASD** (Active Stream Depth), the professor's name
  for the signal: module `algorithms::asd`, name string `asd`, id `0xff01`.
- `eeasd` is now **BACKPRESSURE** (Within-tier Active Stream Depth): it counts only
  the streams at the tier being delivered. Module `algorithms::backpressure`, name
  string `backpressure`, id `0xff04`. Its tuning key is `backpressure.reads` (formerly
  `eeasd.reads`).
The ids on the wire are unchanged.

Names are cosmetic on the wire (negotiation is by id), but the relay's log
lines now read `SSTS asd:` and `SSTS backpressure:`. The former ASD v1.1 id
(`0xff02`) is retired and unused.
