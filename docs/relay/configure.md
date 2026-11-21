---
description: Write a configuration file for the relay, and change the settings you most often need, from the bind address and cache to subscriber limits, logs and qlog traces.
---

# Configure the relay

Use a configuration file to change how the relay runs. This page covers the settings you are most likely to change. [Configuration file](/relay/reference/config) lists every key with its default.

## Write and pass a config file

1. Create a YAML file, for example `relay.yaml`, with only the keys you change. Every key you leave out keeps its default:

   ```yaml
   cache_expiration: 10m
   delivery_timeout: 2s
   ```

   Durations need a unit: `500ms`, `10s`, `30m`, `1h`. Relative paths resolve against the folder you start the relay in, not the folder of the file.

2. Start the relay with it:

   ```sh
   relay --config relay.yaml
   ```

   `--bind`, `--cert-file` and `--key-file` override the same keys in the file. To pass the file to the container image, see [Install the relay](/relay/install).

3. Check that the relay started. An unknown key, or a duration without a unit, stops it at startup with an error that names the key:

   ```
   error: unknown field: found `cache_sise`, expected `one of `bind`, `cert_file`, ...
   ```

## Change the address and port

When the relay should listen on IPv4 only, on one interface or on another port, set `bind` in `relay.yaml`:

```yaml
bind: 0.0.0.0:4433
```

The default, `[::]:4433`, accepts IPv4 and IPv6 on every interface. `0.0.0.0:4433` accepts IPv4 only, and `127.0.0.1:4433` only connections from the same machine. With another port, clients put it in their URL, as in `moqt://relay.example.com:8443`. The `--bind` flag sets the same value.

## Keep more or less in the cache

When subscribers fetch further back than the cache reaches, or the relay should use less memory, set the cache keys in `relay.yaml`:

```yaml
cache_size: 300
cache_expiration_type: tti
cache_expiration: 5m
```

The relay caches up to `cache_size` groups of each track, 1000 by default and 65535 at most. With `ttl`, the default, a group leaves the cache `cache_expiration` after it was cached (30 minutes by default). With `tti`, it leaves `cache_expiration` after it was last used, so groups that subscribers keep fetching stay. A publisher can set a shorter time for its own track with the MAX_CACHE_DURATION track property.

## Limit slow subscribers

When subscribers on poor networks fall behind, and staying close to live matters more than receiving every object, set limits in `relay.yaml`:

```yaml
delivery_timeout: 2s
max_subscriber_queue_bytes: 4194304
max_subscriber_lag: 2000
```

- `delivery_timeout`: when an object has waited in a subscriber's queue this long, the relay skips the rest of its group for that subscriber. A subscriber can ask for a shorter timeout, but not a longer one. The default, `0s`, sets no limit.
- `max_subscriber_queue_bytes`: when this many payload bytes are queued for one subscriber, the relay skips the rest of the group for it. The default is 16 MiB; `0` removes the limit.
- `max_subscriber_lag`: when this many objects are queued for one subscriber, the relay resets its data streams with TOO_FAR_BEHIND. The default, `0`, turns the check off.

## Shed load

When a busy relay should turn new requests away rather than slow down for everyone, set a limit in `relay.yaml`:

```yaml
max_active_requests: 5000
```

Once this many request streams are being served across the relay, it resets new ones with EXCESSIVE_LOAD. The default, `0`, turns this off. Separately, `max_request_streams` (10000 by default) limits the request streams one peer may have open at a time.

## Log objects and tokens

The relay's own log goes to standard output, and `RUST_LOG` sets its level: see [Run the relay in production](/relay/production). When you also need a record of what arrived at the relay and what each subscriber received, or of the tokens clients register, turn on the logs in `relay.yaml`:

```yaml
log_folder: /var/log/moqtail
enable_object_logging: true
enable_token_logging: true
token_log_path: /var/log/moqtail/tokens.csv
```

- `enable_object_logging` writes comma-separated files into `log_folder`, one line per object: `track_<id>.log` for each track the relay receives, `sub_<alias>_<connection>.log` for each subscription and `fetch_<alias>_<connection>_<request>.log` for each fetch. The relay appends to a file for every object, so keep this for tests.
- `enable_token_logging` appends a line to `token_log_path` for each AUTHORIZATION_TOKEN a client registers in its SETUP: the token in hex, the connection ID, the client's port and the Unix time in seconds. The folder must exist. The file holds the tokens themselves, so keep it private.

Whether object logging is on or not, the relay appends a line to `cache_eviction.log` in `log_folder` for every group that leaves the cache. The default `log_folder` is `/tmp`.

## Record qlog traces

When you need the control messages a connection exchanged, for example to see why a client's request failed, set a folder in `relay.yaml`:

```yaml
qlog_folder: /var/log/moqtail/qlog
qlog_max_file_bytes: 10485760
```

The relay writes one qlog file per connection, `server_conn<id>.sqlog`, in JSON-SEQ. A long-lived publisher's file grows for as long as it stays connected. `qlog_max_file_bytes` stops each file at about that size, here 10 MiB, with a last event that says so. The default, `0`, sets no limit. Debug builds also write a QUIC packet trace for each connection, `server_<time>_<address>_quic.sqlog`. The HTTP API can serve the traces: see [Monitor the relay](/relay/observability).

## Give every subscriber of a track the same alias

When you test that a subscriber handles shared track aliases, or want one alias per track in your logs, set the mode in `relay.yaml`:

```yaml
track_alias_mode: per-track
```

By default, `per-subscription`, each subscription gets a new Track Alias, unique within the subscriber's session. With `per-track`, every subscription to a track gets the same alias, the relay's ID for the track. MOQT allows both, so a subscriber has to handle either.

## More information

- [Configuration file](/relay/reference/config): every key, its default and the startup checks
- [Relay CLI](/relay/reference/cli): `--config`, `--bind`, `--cert-file` and `--key-file`
- [Monitor the relay](/relay/observability), for the `api:` section
- [Build a relay mesh](/relay/clustering), for `hop_id`, `peers` and `relay_cost`
- [Run the relay in production](/relay/production), for `RUST_LOG`, sockets and buffers
