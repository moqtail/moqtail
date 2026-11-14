---
'relay': patch
'relay-api': patch
'dashboard': patch
---

A track remembers the connection that last stopped publishing it, as `last_publisher` in the API, and the track page links to it. A cached track whose publisher has gone still leads to that publisher's connection page and qlog trace.
