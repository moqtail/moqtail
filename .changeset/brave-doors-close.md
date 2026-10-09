---
'moqtail': patch
---

Close the WebTransport session in `disconnect()`: the guard tested the `closed` Promise, so `close()` was never called and the session stayed open at the relay
