---
'moqtail-ssts': minor
---

Keep the object clock off the group-based streaks. ASD and ASD v1.1 now
walk their streaks only at a group boundary or a stream timeout: the
object-paced mid-group sample re-evaluates the group in flight but no
longer fills a five-group upshift streak several times inside one group.
backpressure: a clean boundary clears the monitor's mid-group streak, so a lone
deep reading cannot carry across a boundary - or the end of exploration -
into a spurious fire; the UPSHIFT log reports the streak the climb
actually waited (one while exploring, `U(k)` after) instead of the
computed `U(k)` always; and `U(k)` is the maximum over all active sets,
so a connection serving several ladders climbs as cautiously as its most
demanding one.
