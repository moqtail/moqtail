---
'moqtail': patch
---

Serialize the FETCH header on outgoing fetch streams, which was written as an object instead of bytes; report outgoing data through `onDataSent`, which was never wired; and stop reporting each incoming data stream header twice through `onDataReceived`
