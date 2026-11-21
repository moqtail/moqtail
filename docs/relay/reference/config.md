---
description: The keys of the relay's YAML configuration file, their defaults and the rules for writing them.
---

# Configuration file

The relay reads its settings from a YAML file given with `--config`. Without one it runs on the defaults below.

## Rules

- Every key is optional. A key left out takes its default.
- An unknown key stops the relay at startup.
- Durations need a unit: `500ms`, `10s`, `30m`, `1h`.
- Relative paths resolve against the directory the relay starts in, not the directory of the file.
- The flags `--bind`, `--cert-file` and `--key-file` override the file.

The relay also refuses to start when:

| Setting                 | Refused when                                                       |
| ----------------------- | ------------------------------------------------------------------ |
| `dedup_retained_groups` | above 1000                                                         |
| `ssts_algorithms`       | it lists an id the relay does not implement                        |
| `peers`                 | it is not empty and `hop_id` is not set                            |
| `api`                   | see [startup refusals](/relay/reference/http-api#startup-refusals) |

`io_sockets` is not refused: values outside 1 to 256 are clamped.

## Keys

This is the file `apps/relay/relay.example.yaml` from the repository. It lists every key with its default and a short description. Commented-out keys have no default; uncomment one to set it.

<<< ../../../apps/relay/relay.example.yaml

The field documentation on `AppConfig` in [`apps/relay/src/server/config.rs`](https://github.com/moqtail/moqtail/blob/main/apps/relay/src/server/config.rs) has the longer descriptions.
