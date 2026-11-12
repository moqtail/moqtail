---
'moqtail-rs': minor
'relay': minor
'client': minor
---

qlog traces of control messages. Set `qlog_folder` (or `--qlog-folder`) on the relay, or pass `--qlog-folder` to the client, to write one JSON-SEQ `.sqlog` file per connection with a `moqt:control_message_created`/`_parsed` event (decoded message plus raw bytes) for every control message, in the MoQT qlog event format. Unset, nothing is recorded. The per-message hex dumps at INFO level are removed. The relay logs to stdout only; it no longer writes a rolling `relay.log` into `log_folder`.

In debug builds the relay also writes a QUIC packet-level qlog per connection (`*_quic.sqlog`: packets sent, received and lost, RTT and congestion window) into the same folder.
