# MOQtail Test Client

A unified test client for the MOQtail relay, supporting publish, subscribe, and fetch operations over MoQ Transport (draft-18). Supports both subgroup (unidirectional streams) and datagram object forwarding preferences.

## Usage

```
cargo run --bin client -- --command <COMMAND> [OPTIONS]
```

## Commands

| Command             | Description                                        |
| ------------------- | -------------------------------------------------- |
| `publish`           | Publish objects to a track                         |
| `publish-namespace` | Publish a namespace and auto-respond to subscribes |
| `subscribe`         | Subscribe to a track and receive objects           |
| `subscribe-tracks`  | Subscribe to all tracks under a namespace prefix   |
| `fetch`             | Fetch specific object ranges from a track          |

## Global Options

| Option                 | Short | Default                 | Description                                     |
| ---------------------- | ----- | ----------------------- | ----------------------------------------------- |
| `--command`            | `-c`  | _(required)_            | Command to run                                  |
| `--server`             | `-s`  | `moqt://127.0.0.1:4433` | Server address as a `moqt://` URI               |
| `--transport`          |       | `web-transport`         | Session transport (`web-transport` or `quic`)   |
| `--namespace`          | `-n`  | `moqtail`               | Track namespace                                 |
| `--track-name`         | `-T`  | `demo`                  | Track name                                      |
| `--no-cert-validation` |       | `false`                 | Skip certificate validation                     |
| `--delivery-mode`      |       | `subgroup`              | Object delivery mode (`subgroup` or `datagram`) |

## Publish Options

| Option                 | Short | Default    | Description                                                           |
| ---------------------- | ----- | ---------- | --------------------------------------------------------------------- |
| `--group-count`        |       | `100`      | Number of groups to send                                              |
| `--interval`           | `-i`  | `1000`     | Interval between objects in milliseconds                              |
| `--objects-per-group`  |       | `10`       | Number of objects per group                                           |
| `--object-id-step`     |       | `1`        | Stride between object IDs (`2` → 0, 2, 4 …); sets Prior Object ID Gap |
| `--group-id-step`      |       | `1`        | Stride between group IDs (`2` → 0, 2, 4 …); sets Prior Group ID Gap   |
| `--payload-size`       |       | `1200`     | Payload size in bytes                                                 |
| `--track-alias`        |       | _(random)_ | Track alias (random if not specified)                                 |
| `--publisher-priority` |       | `128`      | Publisher priority 0 (highest) – 255 (lowest)                         |

## Subscribe Options

| Option                   | Short | Default     | Description                                                                |
| ------------------------ | ----- | ----------- | -------------------------------------------------------------------------- |
| `--duration`             | `-d`  | `0`         | Duration to listen in seconds (0 = indefinite)                             |
| `--subscriber-priority`  |       | `128`       | Subscriber priority 0 (highest) – 255 (lowest)                             |
| `--group-order`          |       | `ascending` | Group delivery order (`original`, `ascending`, `descending`)               |
| `--extra-track`          |       | _(none)_    | Second track subscription for priority testing: `name:priority`            |
| `--forward`              |       | `true`      | Subscription Forward State; set `false` to suppress object delivery        |
| `--update-forward-after` |       | `0`         | Seconds after subscribing to REQUEST_UPDATE Forward State to 1 (0 = never) |
| `--joining-fetch`        |       | `false`     | After subscribing, issue a Joining FETCH against the subscription          |
| `--joining-start`        |       | `0`         | Joining FETCH start group (with `--joining-fetch`)                         |
| `--joining-type`         |       | `relative`  | Joining FETCH type (`relative` or `absolute`)                              |

## Fetch Options

| Option           | Default | Description                                          |
| ---------------- | ------- | ---------------------------------------------------- |
| `--start-group`  | `1`     | Start group ID                                       |
| `--start-object` | `0`     | Start object ID                                      |
| `--end-group`    | `5`     | End group ID                                         |
| `--end-object`   | `3`     | End object ID                                        |
| `--cancel-after` | `0`     | Cancel the fetch after receiving N objects (0 = off) |

## Examples

Publish 50 groups of datagrams at 500ms intervals:

```
cargo run --bin client -- -c publish --delivery-mode datagram --group-count 50 --interval 500
```

Subscribe to datagrams for 30 seconds:

```
cargo run --bin client -- -c subscribe --delivery-mode datagram --duration 30
```

Subscribe to subgroup streams indefinitely:

```
cargo run --bin client -- -c subscribe
```

Fetch objects from groups 0-10:

```
cargo run --bin client -- -c fetch --start-group 0 --end-group 10
```

Publish a track with gaps in the object IDs (0, 2, 4 …), then FETCH it — the
response delivers only the existing objects and closes with a FIN, so the gaps
signal non-existent objects. Each skipped object also carries the Prior Object
ID Gap property (draft 12.9), which the fetcher logs as `[prior_object_gap=N]`:

```
cargo run --bin client -- -c publish --object-id-step 2 --objects-per-group 3
cargo run --bin client -- -c fetch --start-group 0 --end-group 3
```

Group-ID gaps work the same way via `--group-id-step`, setting the Prior Group
ID Gap property (draft 12.8) on the first object of each gapped group. Run the
relay with `--max-upstream-fetch-gaps 0` so it serves only the cached groups
instead of trying to fetch the (non-existent) missing groups upstream:

```
cargo run --bin client -- -c publish --group-id-step 2 --objects-per-group 2
cargo run --bin client -- -c fetch --start-group 0 --end-group 7
```

Subscribe, then issue a Joining FETCH for the last 2 groups (a Joining FETCH is
only accepted when the subscription is forwarding):

```
cargo run --bin client -- -c subscribe --joining-fetch --joining-start 2
```

Same, but with a non-forwarding subscription — the relay rejects the Joining
FETCH with `REQUEST_ERROR` / `INVALID_RANGE`:

```
cargo run --bin client -- -c subscribe --forward false --joining-fetch --joining-start 2
```

Subscribe with Forward State 0 (no objects), then flip it to 1 after 3 seconds —
the relay reopens the subgroup and delivery resumes:

```
cargo run --bin client -- -c subscribe --forward false --update-forward-after 3 --duration 8
```

Subscribe to every track under a namespace prefix (SUBSCRIBE_TRACKS). The relay
answers `REQUEST_OK`, forwards a `PUBLISH` for each matching track, and sends
`PUBLISH_BLOCKED` if it runs out of streams (start the relay with
`--max-publish-streams N` to cap it):

```
cargo run --bin client -- -c subscribe-tracks -n test/ns --duration 5
```

Connect to a remote server with certificate validation disabled:

```
cargo run --bin client -- -c publish --server moqt://relay.example.com:4433 --no-cert-validation
```

Connect over raw QUIC instead of WebTransport:

```
cargo run --bin client -- -c subscribe --server moqt://relay.example.com:4433 --transport quic
```

## Testing QUIC Stream Priority Scheduling

The relay implements the MOQT four-tier scheduling algorithm: subscriber priority > publisher priority > group order > subgroup/object ID. QUIC stream priorities are assigned using a band mapping so tracks with different priorities never interfere on the same connection.

To observe the scheduler in action you need bandwidth pressure on a single subscriber connection. The relay's `--write-kbps-limit` flag creates a per-connection token bucket that throttles writes, causing the QUIC send buffer to fill and forcing the scheduler to choose between streams.

**Terminal 1 — relay with 200 kbps per-subscriber cap:**

```
cargo run --bin relay -- --write-kbps-limit 200
```

**Terminal 2 — high-priority publisher (track `demo`, publisher-priority=128):**

```
cargo run --bin client -- -c publish-namespace -T demo \
  --publisher-priority 128 --no-cert-validation \
  --interval 0 --group-count 80 --objects-per-group 2 --payload-size 30000
```

**Terminal 3 — low-priority publisher (track `demo2`, publisher-priority=128):**

```
cargo run --bin client -- -c publish-namespace -T demo2 \
  --publisher-priority 128 --no-cert-validation \
  --interval 0 --group-count 80 --objects-per-group 2 --payload-size 30000
```

**Terminal 4 — one subscriber to both tracks with different subscriber priorities:**

```
cargo run --bin client -- -c subscribe \
  -T demo --subscriber-priority 10 \
  --extra-track "demo2:200" \
  --no-cert-validation --duration 90
```

Start the publishers before the subscriber. The subscriber log will show objects from both tracks interleaved. At every group where both tracks deliver simultaneously, `demo` (priority 10) consistently arrives before `demo2` (priority 200):

```
Received object 10: track=alias=1(primary) group=2, object=1  ← arrives first
Received object 11: track=alias=2(demo2)   group=2, object=0  ← arrives 0.3 ms later
...
Received object 30: track=alias=1(primary) group=7, object=1  ← arrives first
Received object 31: track=alias=2(demo2)   group=7, object=0  ← arrives 0.7 ms later
```

The `seq=GAP` messages in the subscriber output are expected — the stats counter tracks objects from both tracks as a single sequence, so cross-track interleaving appears as gaps. They do not indicate missing objects.

### Priority parameters

| Parameter               | Where set      | Effect                                                                      |
| ----------------------- | -------------- | --------------------------------------------------------------------------- |
| `--subscriber-priority` | subscriber CLI | Most significant priority tier (0 = highest)                                |
| `--publisher-priority`  | publisher CLI  | Second tier, tie-breaks within same subscriber priority                     |
| `--group-order`         | subscriber CLI | `ascending`: earlier groups preferred; `descending`: later groups preferred |
| `--extra-track name:N`  | subscriber CLI | Subscribes to a second track at priority N on the same connection           |

### Using OS-level bandwidth throttling

For a real UDP-layer bottleneck, throttle the loopback interface at the OS level and run the relay **without** `--write-kbps-limit`. Any tool that limits UDP throughput to roughly 1 Mbit/s is sufficient.

**Linux — `tc` (traffic control):**

```bash
# Add a 1 Mbit/s rate limit with 10 ms delay on loopback
sudo tc qdisc add dev lo root netem rate 1mbit delay 10ms

# Run tests…

# Teardown
sudo tc qdisc del dev lo root
```

**macOS — Network Link Conditioner:**
Install the **Network Link Conditioner** preference pane from _Additional Tools for Xcode_ (available at developer.apple.com/download/all). Create a custom profile with ~1000 Kbps downlink/uplink and enable it. Disable when done.
