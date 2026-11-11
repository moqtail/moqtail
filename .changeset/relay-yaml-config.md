---
'relay': minor
---

Move the relay's settings into a YAML file passed with `--config` (see `apps/relay/relay.example.yaml`). The command line keeps only `--config`, `--bind`, `--cert-file` and `--key-file`, which override the file. `--host`/`--port` are now `--bind <ip:port>`, and durations take units (`10s`, `500ms`, `30m`), so the `_secs`/`_ms`/`_minutes` suffixes are gone from the keys.
