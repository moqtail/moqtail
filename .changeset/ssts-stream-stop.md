---
'moqtail-rs': minor
---

`RecvDataStream::stop_sending(code)` lets a caller ask the peer to stop a data
stream without consuming the read half, so a client can cancel while another
task is blocked reading. `TransportRecvStream::stop_by_ref` is the shared-handle
form of `stop`.
