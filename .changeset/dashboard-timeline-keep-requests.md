---
'dashboard': patch
---

A long-lived connection's message timeline keeps its requests. Past 20,000 events the oldest stream headers are dropped instead of the oldest events, which took the control messages with them a few seconds after the page opened.
