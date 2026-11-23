---
'moqtail-rs': minor
---

`RecvDataStream::stop_sending(code)` asks the peer to stop a data stream
without consuming the read half, so a client can cancel while another task is
blocked in `next_object()`. `TransportRecvStream::stop_by_ref` is the
shared-handle form of `stop`, for callers that hold the stream behind a mutex.
