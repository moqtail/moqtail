# Backpressure — within-tier depth, decided inside the group

**Backpressure** counts the depth signal only at the rung actually being
delivered, reads it again while a group is still in flight, and can reach back
into the groups a downshift overtook. It is the within-tier successor to the
relay's earlier connection-wide depth controller.

It exists because the earlier algorithms share an assumption: that a group is
the unit of time. Groups are the unit the _media_ offers, but the path changes
faster than 25 groups a second, and by the time a group boundary arrives the
decision is already late. They also read the whole connection queue as
congestion, which makes recovery from a downshift look like the problem the
downshift was meant to fix.

## Identity

Name: `backpressure`, wire id `0xff04`
(`PRIVATE_ALGORITHM_ID_BASE + 3`). It is one algorithm and one id. The
retroactive supersede is a toggle in the assignment extension, not a separate
algorithm id.

The extension fields follow the subscriber's latency budget:

| Position | Field                   | Meaning                                                                    |
| -------- | ----------------------- | -------------------------------------------------------------------------- |
| 0        | `rho_pct`               | Probing risk tolerance, 0 default, 1–100                                   |
| 1        | `U_min`                 | Floor under the clean-group wait; 0 is the draft default                   |
| 2        | `U_max`                 | Cap on the wait; 0 means unbounded                                         |
| 3        | `retroactive_supersede` | 0/absent: classic; non-zero: cancel and re-send the newest overtaken group |

## The toggle

On a downshift from 1080p to 720p at group 4, with groups 1–3 still in flight
at 1080p:

- **off** — group 4 goes out at 720p; groups 1–3 finish at 1080p.
- **on** — group 3, the newest in flight above the new rung, is cancelled and
  re-sent at 720p; groups 1 and 2 finish where they are.

The re-sent stream is excluded from the depth, so the repair does not read as
new congestion. A repair in flight still vetoes a climb until it resolves.

## What it measures

The depth is the number of live forwarding streams carrying, for each active
set, the member at the tier the last decision chose. Streams at other rungs
belong to a quality already rejected; streams opened to re-send an overtaken
group are the relay's own doing. Both exclusions are load-bearing, or the
recovery from a downshift looks like the congestion it was meant to fix.

The climb is also held while an old stream is still open at the rung above:
climbing onto it would make the next group's stream the second one there and
the monitor would read the climb itself as congestion.

## Tuning

Operator keys, read once per connection:

- `backpressure.reads` — mid-group monitor patience, default 2
- `backpressure.rho` — default probing risk tolerance
- `backpressure.umin` / `backpressure.umax` — default rails

The subscriber's extension fields override the operator defaults; a missing
field falls back to the operator value.

## Known shortcomings

- A short dip can leave a queue behind: the drain overlaps the next group and
  the monitor can read it as a second congestion.
- The supersede is newest-only. Older overtaken groups finish on the member
  they were carrying even when their delivery window is nearly gone.
- Only one switching set may be assigned to the family per connection. Other
  sets on the connection may use an allocation algorithm such as the default.
