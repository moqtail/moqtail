---
'moqtail-rs': minor
'relay': minor
'relay-api': minor
'dashboard': minor
---

qlog traces record how each data stream ended (FIN, RESET_STREAM or STOP_SENDING, with the error code) and why the connection closed, as `moqtail:stream_end_created`, `moqtail:stream_end_parsed` and `moqtail:connection_closed`. The message timeline in the dashboard and in the MCP `connection_timeline` tool shows streams cut short and the close, and counts FINs per track. `TransportWriteError` gains `Stopped(code)` for a stream the peer stopped with STOP_SENDING.
