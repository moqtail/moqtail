---
'client': minor
'moqtail': minor
---

React to GOAWAY instead of waiting for the relay to close the session. The Rust test client logs the GOAWAY's new session URI and timeout, stops its command and closes the session; subscribe and publish commands then reconnect to the new session URI and run again, and leave when none is given. The player leaves the session on GOAWAY and reopens it at the new session URI (or the current one when none is given), so players see `reconnecting` and `reconnected` as for a lost session.
