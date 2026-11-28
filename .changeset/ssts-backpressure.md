---
'moqtail-ssts': minor
'relay': minor
---

Add the SSTS Backpressure controller at private algorithm id `0xff04`.

Backpressure measures active stream depth at the tier currently being
delivered and uses it to drive fast start, risk-scaled steady-state upshifts
and intra-group downshifts. A subscriber can signal its latency budget,
probing risk tolerance, upshift bounds and whether an overtaken group should
be re-sent retroactively.

The relay advertises the controller as opt-in, validates switching-set
assignments against the algorithms it runs, tracks per-group forwarding
decisions, and replaces an overtaken group from the relay cache when the
replacement can start at the beginning of the subgroup. Only one switching
set may be assigned to Backpressure per connection; other sets may use the
default allocation algorithm.

The crate also adds ASD as the legacy stream-depth reference and a
perfect-knowledge oracle for testbed comparisons. Their private ids are
implementation-assigned and may change before registration.

The connection-wide `ssts_discard_timeout` setting is removed; discard
deadlines now come only from the switching set's negotiated latency budget.
Datagrams on switching-set tracks are not forwarded yet and are dropped with
one debug line per subscription.
