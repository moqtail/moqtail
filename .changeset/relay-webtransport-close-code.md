---
'relay': patch
---

Close WebTransport sessions with a CLOSE_WEBTRANSPORT_SESSION capsule, so browser peers see the error code (such as PROTOCOL_VIOLATION) instead of "Connection lost" with code 0.
