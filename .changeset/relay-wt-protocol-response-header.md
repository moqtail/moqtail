---
'relay': patch
---

Send the negotiated WebTransport subprotocol back to the client. The relay built the `wt-protocol` response header but then called `accept()`, which discards it — the extended-CONNECT 200 response carried no `wt-protocol` header at all, and the value was also formatted as a bare token instead of the RFC 8941 quoted sf-string form. Clients that require the subprotocol echo fail version negotiation against the relay. The relay now replies via `accept_with_headers()` with the selected version in quoted form (e.g. `"moqt-16"`), matching the quoted items the client offered in `wt-available-protocols`.
