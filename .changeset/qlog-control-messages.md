---
'moqtail-rs': minor
'relay': minor
'client': minor
---

qlog traces. Set `qlog_folder` in the relay config, or pass `--qlog-folder` to the client, to write one JSON-SEQ `.sqlog` file per connection in the MoQT qlog event format; unset, nothing is recorded. A trace holds a `moqt:control_message_created`/`_parsed` event (decoded message plus raw bytes) for every control message, a `moqt:subgroup_header_*`/`moqt:fetch_header_*` event for every data stream opened or accepted (one per stream, not per object), how each data stream ended (`moqtail:stream_end_created`/`_parsed`: FIN, RESET_STREAM or STOP_SENDING, with the error code) and why the connection closed (`moqtail:connection_closed`). The relay names its files `server_conn<id>.sqlog` and never overwrites an earlier trace: an id seen again after a restart gets a `_2`, `_3`, … suffix. `qlog_max_file_bytes` stops a file at about that size with a last `moqtail:trace_truncated` event; 0, the default, means no limit, and `qlog::init` takes it as a third argument. In debug builds the relay also writes a QUIC packet-level qlog per connection (`*_quic.sqlog`: packets sent, received and lost, RTT and congestion window). `scripts/qlog-summary.py <folder>` prints each connection's peer SETUP, requests with their responses, data streams per track and parse errors.

The per-message hex dumps at INFO level are removed, and the relay logs to stdout only; it no longer writes a rolling `relay.log` into `log_folder`. `TransportWriteError` gains `Stopped(code)` for a stream the peer stopped with STOP_SENDING.
