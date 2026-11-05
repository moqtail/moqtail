---
'moqtail': minor
---

Order media per request in the player. The `objectOrdering` player option sets how CMAF sources order their objects, by default a 150 ms window. After an `orderinggap` a source drops the rest of that group and resumes at the next group with a `gap` discontinuity. Catalog and timeline readers keep their own ordering. During a switch the client lets no ordering window expire, and holds the new track's ordered objects until the track it switched from is retired, so a soft switch's drain comes first.
