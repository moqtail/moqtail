# MOQtail Test Client

A unified test client for the MOQtail relay, supporting publish, subscribe, and fetch operations over MoQ Transport (draft-14). Supports both subgroup (unidirectional streams) and datagram object forwarding preferences.

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
| `fetch`             | Fetch specific object ranges from a track          |

## Global Options

| Option                    | Short | Default                  | Description                                                  |
| ------------------------- | ----- | ------------------------ | ------------------------------------------------------------ |
| `--command`               | `-c`  | _(required)_             | Command to run                                               |
| `--server`                | `-s`  | `https://127.0.0.1:4433` | Server address                                               |
| `--namespace`             | `-n`  | `moqtail`                | Track namespace                                              |
| `--track-name`            | `-T`  | `demo`                   | Track name                                                   |
| `--no-cert-validation`    |       | `false`                  | Skip certificate validation                                  |
| `--forwarding-preference` |       | `subgroup`               | Object forwarding preference (`subgroup` or `datagram`)      |
| `--group-order`           |       | `ascending`              | Group delivery order (`original`, `ascending`, `descending`) |

## Publish Options

| Option                 | Short | Default    | Description                                   |
| ---------------------- | ----- | ---------- | --------------------------------------------- |
| `--group-count`        |       | `100`      | Number of groups to send                      |
| `--interval`           | `-i`  | `1000`     | Interval between objects in milliseconds      |
| `--objects-per-group`  |       | `10`       | Number of objects per group                   |
| `--payload-size`       |       | `1200`     | Payload size in bytes                         |
| `--track-alias`        |       | _(random)_ | Track alias (random if not specified)         |
| `--publisher-priority` |       | `128`      | Publisher priority 0 (highest) – 255 (lowest) |

## Subscribe Options

| Option                  | Short | Default  | Description                                                     |
| ----------------------- | ----- | -------- | --------------------------------------------------------------- |
| `--duration`            | `-d`  | `0`      | Duration to listen in seconds (0 = indefinite)                  |
| `--subscriber-priority` |       | `128`    | Subscriber priority 0 (highest) – 255 (lowest)                  |
| `--extra-track`         |       | _(none)_ | Second track subscription for priority testing: `name:priority` |

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
cargo run --bin client -- -c publish --forwarding-preference datagram --group-count 50 --interval 500
```

Subscribe to datagrams for 30 seconds:

```
cargo run --bin client -- -c subscribe --forwarding-preference datagram --duration 30
```

Subscribe to subgroup streams indefinitely:

```
cargo run --bin client -- -c subscribe
```

Fetch objects from groups 0-10:

```
cargo run --bin client -- -c fetch --start-group 0 --end-group 10
```

Connect to a remote server with certificate validation disabled:

```
cargo run --bin client -- -c publish --server https://relay.example.com:4433 --no-cert-validation
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

| Parameter               | Where set                | Effect                                                                      |
| ----------------------- | ------------------------ | --------------------------------------------------------------------------- |
| `--subscriber-priority` | subscriber CLI           | Most significant priority tier (0 = highest)                                |
| `--publisher-priority`  | publisher CLI            | Second tier, tie-breaks within same subscriber priority                     |
| `--group-order`         | publisher/subscriber CLI | `ascending`: earlier groups preferred; `descending`: later groups preferred |
| `--extra-track name:N`  | subscriber CLI           | Subscribes to a second track at priority N on the same connection           |

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
