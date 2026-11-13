---
'relay': patch
---

A relay now acts on GOAWAY from a cluster peer: it closes the link at once, so routes through the peer are withdrawn within moments instead of after the idle timeout, and it redials the new URI the GOAWAY names, or the same peer once the GOAWAY's timeout has passed. The peer link in `/api/v1/cluster` shows "peer sent GOAWAY" as the reason.
