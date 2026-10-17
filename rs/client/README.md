# MOQtail Test Client

A unified test client for the MOQtail relay, supporting publish, subscribe, and fetch operations over MoQ Transport (draft-22). Supports both subgroup (unidirectional streams) and datagram object forwarding preferences.

## Usage

```
cargo run --bin client -- --command <COMMAND> [OPTIONS]
```

## Commands

| Command               | Description                                        |
| --------------------- | -------------------------------------------------- |
| `publish`             | Publish objects to a track                         |
| `publish-namespace`   | Publish a namespace and auto-respond to subscribes |
| `subscribe`           | Subscribe to a track and receive objects           |
| `subscribe-namespace` | Discover the namespaces under a prefix             |
| `subscribe-tracks`    | Subscribe to all tracks under a namespace prefix   |
| `fetch`               | Fetch specific object ranges from a track          |

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

| Option                  | Short | Default    | Description                                                                                                  |
| ----------------------- | ----- | ---------- | ------------------------------------------------------------------------------------------------------------ |
| `--group-count`         |       | `100`      | Number of groups to send                                                                                     |
| `--interval`            | `-i`  | `1000`     | Interval between objects in milliseconds                                                                     |
| `--objects-per-group`   |       | `10`       | Number of objects per group                                                                                  |
| `--object-id-step`      |       | `1`        | Stride between object IDs (`2` → 0, 2, 4 …); sets Prior Object ID Gap                                        |
| `--group-id-step`       |       | `1`        | Stride between group IDs (`2` → 0, 2, 4 …); sets Prior Group ID Gap                                          |
| `--payload-size`        |       | `1200`     | Payload size in bytes                                                                                        |
| `--track-alias`         |       | _(random)_ | Track alias (random if not specified)                                                                        |
| `--publisher-priority`  |       | `128`      | Publisher priority 0 (highest) – 255 (lowest)                                                                |
| `--subgroups-per-group` |       | `1`        | Split each group over N subgroups, Object _i_ going to subgroup _i_ mod N, each on its own stream            |
| `--priorities`          |       | _(none)_   | Publisher priority of each subgroup in turn, e.g. `10,200`; a datagram takes its would-be subgroup's         |
| `--object-property`     |       | _(none)_   | `TYPE=SOURCE` set on every Object (repeatable); SOURCE is `group`, `object`, `subgroup`, `mod:M` or a number |
| `--track-property`      |       | _(none)_   | `TYPE=VALUE` advertised in PUBLISH, SUBSCRIBE_OK, FETCH_OK and TRACK_STATUS_OK (repeatable)                  |

## Subscribe Options

| Option                   | Short | Default       | Description                                                                                                                     |
| ------------------------ | ----- | ------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `--duration`             | `-d`  | `0`           | Duration to listen in seconds (0 = indefinite)                                                                                  |
| `--subscriber-priority`  |       | `128`         | Subscriber priority 0 (highest) – 255 (lowest)                                                                                  |
| `--group-order`          |       | `ascending`   | Group delivery order (`original`, `ascending`, `descending`)                                                                    |
| `--extra-track`          |       | _(none)_      | Second track subscription for priority testing: `name:priority`                                                                 |
| `--forward`              |       | `true`        | Subscription Forward State; set `false` to suppress object delivery                                                             |
| `--include-properties`   |       | `true`        | `false` sends INCLUDE_PROPERTIES=0: no Track Properties in the SUBSCRIBE_OK, FETCH_OK or pushed PUBLISH                         |
| `--update-forward-after` |       | `0`           | Seconds after subscribing to REQUEST_UPDATE Forward State to 1 (0 = never)                                                      |
| `--filter`               |       | `next-object` | Location Filter: `none`, `relative-start`, `absolute-start`, `absolute-start-group-end`, `absolute-range`, `next-object`        |
| `--filter-start-group`   |       | `0`           | Start Group for the absolute filters; for `relative-start`, groups before the next group (0 = next, 1 = current)                |
| `--filter-start-object`  |       | `0`           | Start Object for the absolute filters                                                                                           |
| `--end-group-delta`      |       | `0`           | End Group as groups after the Start Group (`absolute-start-group-end`, `absolute-range`)                                        |
| `--filter-end-object`    |       | `0`           | Last Object of the End Group (`absolute-range`)                                                                                 |
| `--fill`                 |       | `false`       | Also retrieve what is already published over `--filter`'s range, on a fill fetch stream (FILL_PARAMETERS)                       |
| `--fill-start-group`     |       | _(none)_      | Fill from this many groups before the next group instead (1 = the current group); implies `--fill`                              |
| `--switch-after`         |       | `0`           | Seconds after subscribing to switch to `--switch-track` with SWITCH_FROM (0 = never)                                            |
| `--switch-track`         |       | _(none)_      | Track to switch to                                                                                                              |
| `--switch-mode`          |       | `hard`        | `hard` stops the old subscription at the cutover; `soft` lets it drain to the group before the new one's start                  |
| `--switch-publish-done`  |       | `false`       | Ask for PUBLISH_DONE on the suspended subscription                                                                              |
| `--switch-start-group`   |       | `0`           | Switch at this future group instead of where `--filter` starts (0 = use `--filter`); also fills what is already published of it |
| `--range-filter`         |       | _(none)_      | A Range Filter (repeatable; also for `fetch` and `subscribe-tracks`). See [Range Filters](#range-filters)                       |
| `--fill-range-filter`    |       | _(none)_      | A Range Filter for the fill fetch stream, inside FILL_PARAMETERS (repeatable); implies `--fill`                                 |
| `--update-filters-after` |       | `0`           | Seconds after subscribing to send a REQUEST_UPDATE carrying `--update-range-filter` (0 = never)                                 |
| `--update-range-filter`  |       | _(none)_      | A Range Filter for that REQUEST_UPDATE (repeatable); `KIND:clear` removes a kind                                                |
| `--ignore-filter-limit`  |       | `false`       | Send Range Filters past the server's MAX_FILTER_RANGES, to test that it refuses them                                            |

## Fetch Options

The range is sent as an `absolute-range` Location Filter.

| Option           | Default | Description                                          |
| ---------------- | ------- | ---------------------------------------------------- |
| `--start-group`  | `1`     | Start group ID                                       |
| `--start-object` | `0`     | Start object ID                                      |
| `--end-group`    | `5`     | End group ID                                         |
| `--end-object`   | `3`     | Last object ID of the end group, inclusive           |
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

Subscribe from the next group, rather than mid-group from the next object:

```
cargo run --bin client -- -c subscribe --filter relative-start --filter-start-group 0
```

Subscribe from the next object and fill the current group from its start. The
fill arrives on a fill fetch stream (a stream beginning with a FETCH_HEADER named
by the SUBSCRIBE), so the subscriber has the whole group without waiting for the
next one:

```
cargo run --bin client -- -c subscribe --fill-start-group 1
```

Subscribe to groups 10 through 12 only, filling whatever of them is already
published (a filter with an end stops delivery there, but the subscription stays
open):

```
cargo run --bin client -- -c subscribe --filter absolute-start-group-end \
  --filter-start-group 10 --end-group-delta 2 --fill
```

Switch from track `demo` to `demo2` after 5 seconds. A hard switch stops `demo`
as soon as `demo2` delivers; a soft one lets `demo` drain to the group before the
one `demo2` starts at. `--switch-start-group` picks that group, so the two meet
on a group boundary:

```
cargo run --bin client -- -c subscribe -T demo --switch-after 5 --switch-track demo2
cargo run --bin client -- -c subscribe -T demo --switch-after 5 --switch-track demo2 \
  --switch-mode soft --switch-start-group 40 --switch-publish-done
```

Subscribe with Forward State 0 (no objects), then flip it to 1 after 3 seconds —
the relay reopens the subgroup and delivery resumes:

```
cargo run --bin client -- -c subscribe --forward false --update-forward-after 3 --duration 8
```

Discover the namespaces under a prefix (SUBSCRIBE_NAMESPACE). The relay answers
`REQUEST_OK`, then a `NAMESPACE` for each namespace already announced under the
prefix and for each one announced later, and a `NAMESPACE_DONE` when one is
withdrawn:

```
cargo run --bin client -- -c subscribe-namespace -n test/ns --duration 5
```

Subscribe to every track under a namespace prefix (SUBSCRIBE_TRACKS). The relay
answers `REQUEST_OK`, forwards a `PUBLISH` for each matching track — which the
client accepts, then receives and counts that track's objects — and sends
`PUBLISH_SKIPPED` if it runs out of streams (start the relay with
`--max-publish-streams N` to cap it):

```
cargo run --bin client -- -c subscribe-tracks -n test/ns --duration 5
```

To join each pushed track partway, give `--filter` (stated in its `PUBLISH` as
where the subscription starts) and `--fill` (a fill fetch stream of what is
already published, opened as soon as the `PUBLISH` is sent). This starts every
track at its current group and fills that group in; `--forward false` pushes the
tracks paused, with no fill:

```
cargo run --bin client -- -c subscribe-tracks -n test/ns --filter relative-start --filter-start-group 1 --fill --duration 5
```

Both prefix subscriptions live for exactly as long as their request stream, so
`--duration` ends one by resetting that stream: the relay drops the subscription
and frees the prefix while the session stays open. With `--duration 0` the run
lasts until the relay closes the stream.

`--overlap` issues a second prefix subscription on the same prefix and session
once the first is live, which is what shows whether the two are allowed to share
a prefix. The kinds keep separate spaces, so a `SUBSCRIBE_TRACKS` and a
`SUBSCRIBE_NAMESPACE` both succeed:

```
cargo run --bin client -- -c subscribe-tracks -n test/ns --overlap namespace --duration 5
```

while a second subscription of the same kind is refused with `PREFIX_OVERLAP`:

```
cargo run --bin client -- -c subscribe-tracks -n test/ns --overlap tracks --duration 5
```

Connect to a remote server with certificate validation disabled:

```
cargo run --bin client -- -c publish --server moqt://relay.example.com:4433 --no-cert-validation
```

Connect over raw QUIC instead of WebTransport:

```
cargo run --bin client -- -c subscribe --server moqt://relay.example.com:4433 --transport quic
```

## Range Filters

`--range-filter` takes `KIND:SET[:PROPERTY]:RANGES`, or `KIND:clear` for the zero-length
form that removes a kind in a REQUEST_UPDATE:

- `KIND` is `subgroup`, `object`, `priority`, `object-property` or `track-property`
  (the last for `subscribe-tracks` only).
- `SET` is the SetID, 0–255. Filters sharing a SetID are ANDed; SetIDs are ORed.
- `PROPERTY` is the Property Type, for the two property kinds only.
- `RANGES` is a comma-separated list of `A-B`, `A` or `A-` (open-ended, last only), in
  decimal or `0x` hex. Ranges within one filter are ORed.

The client reads the server's MAX_FILTER_RANGES from SETUP (0 when it sends none) and
refuses to send more ranges than that, unless `--ignore-filter-limit` is given to test
that the server refuses them. A filter only has to be encodable to be sent: an odd
Property Type, a priority above 255 or a repeated SetID go out as given, for the server
to refuse with INVALID_FILTER.

Every object received is checked against the filters it was requested with. One that
does not pass is logged as `FILTER_VIOLATION` and counted as `filter_violations` in the
summary; a filtered subscription skips the sequence-gap check, since its gaps are by
design. After a REQUEST_UPDATE the filters it replaced still count for two seconds, for
the objects already in flight. For `subscribe-tracks`, a pushed PUBLISH whose Track
Properties pass no track filter is reported, and its objects are then checked against
the object filters of the sets it does pass.

Publish a track with something to filter on: two subgroups per group at priorities 10
and 200, the Object ID mod 3 as Object Property `0x3c`, and Track Property `0x3c=1`:

```
cargo run --bin client -- -c publish -T rf --subgroups-per-group 2 --priorities 10,200 \
  --object-property 0x3c=mod:3 --track-property 0x3c=1 --interval 100
```

Ask for objects 0–1 of each group, or anything in subgroup 1 at high priority:

```
cargo run --bin client -- -c subscribe -T rf --range-filter object:0:0-1 \
  --range-filter subgroup:1:1 --range-filter priority:1:0-63 --duration 10
```

Ask for Object Property `0x3c` in 0–1, then drop the filter after five seconds:

```
cargo run --bin client -- -c subscribe -T rf --range-filter object-property:0:0x3c:0-1 \
  --update-filters-after 5 --update-range-filter object-property:clear --duration 10
```

Only tracks whose Track Property `0x3c` is 2 (the track above should not be pushed):

```
cargo run --bin client -- -c subscribe-tracks -n moqtail --range-filter track-property:0:0x3c:2 --duration 10
```

Fetch only the high-priority objects:

```
cargo run --bin client -- -c fetch -T rf --start-group 0 --end-group 3 --range-filter priority:0:0-63
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
