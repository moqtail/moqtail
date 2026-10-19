---
'moqtail-ssts': minor
---

A re-send can be overtaken again. A second step down cancels a repair
still in flight above the new rung and re-sends it at the lower one,
and a re-sent stream donates its remaining buffer to the next repair -
its age measured from the moment the repair started, which is exactly
what the second repair has left of the same window. The depth still
excludes re-sends: counting them was built and measured, and on
PROFILE_JITTER a 500 ms dip cost three rungs instead of one - each
repair overlapping the next group manufactured the evidence for the
next step down.
