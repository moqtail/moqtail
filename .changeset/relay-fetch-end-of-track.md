---
'relay': patch
---

A FETCH served from the cache after the publisher ended the track, with PUBLISH_DONE(TRACK_ENDED) or an End of Track Object, now reports End Of Track in FETCH_OK when its range reaches the track's last Object. It used to report 0 whenever the relay answered without asking the publisher. A new publisher of the track clears the end.
