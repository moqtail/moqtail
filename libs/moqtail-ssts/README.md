# moqtail-ssts

Sender-side track switching for [MOQtail](../../README.md): the algorithm
contract, the registry, and the algorithms that ship with it.

The relay owns the mechanism. It keeps the switching sets, forwards the Objects
a decision selects, and applies the result to the streams it owns. This crate
owns the decision, and the only thing that crosses the boundary in either
direction is [`AbrInput`](src/lib.rs) going in and
[`Selection`](src/lib.rs) coming back:

```text
relay                                        moqtail-ssts
-----                                        ------------
subscription gate  -- group, streams ---->   AbrInput
                         (decision) <----    Selection  (set -> track / None)
switching sets                               algorithms, registry
```

Nothing in this crate knows what a `MOQTClient`, a `Track` or an `AppConfig` is,
and `decide` is synchronous: an algorithm is a state machine over the input it
was handed, which is what makes it unit-testable without a transport. The relay
validates the answer it gets back, so a selection naming a set or a track that
does not exist degrades into "forward nothing" with a warning that names the
algorithm, rather than into a stalled stream.

## Adding an algorithm

Three steps, and none of them is in the relay.

1. **Write it.** A new directory under [`src/algorithms`](src/algorithms), with
   the implementation, a factory, its `tests.rs` and, if the behaviour needs
   explaining, a README saying what it is good at and what it is bad at:

   ```rust
   pub struct MyAlgorithmFactory;

   impl AbrAlgorithmFactory for MyAlgorithmFactory {
     fn id(&self) -> u64 { crate::registry::PRIVATE_ALGORITHM_ID_BASE + 1 }
     fn name(&self) -> &'static str { "my-algorithm" }
     fn create(&self) -> Box<dyn AbrAlgorithm> { Box::<MyAlgorithm>::default() }
   }

   impl AbrAlgorithm for MyAlgorithm {
     fn decide(&mut self, input: &AbrInput) -> Selection {
       // answer for every set in `input.sets`, and only with tracks that
       // belong to it
     }
   }
   ```

   `create` returns a fresh instance per connection, so per-connection state is
   a field on the struct: no maps keyed by client id, no locks, and nothing to
   clean up when the connection goes away.

2. **Register it.** One line in [`src/registry.rs`](src/registry.rs). That is
   the whole integration: the SETUP advertisement, the relay's
   `--ssts-algorithms` validation and the instance the controller runs are all
   derived from the registry, so there is no second list to update and no way
   for the two to disagree.

3. **Give it a private id.** `PRIVATE_ALGORITHM_ID_BASE` and above, unless the
   algorithm is being contributed as a standard one. The registry test
   `ids_are_either_standard_or_private` fails an id that sits in the range a
   future standards allocation would land in, so a collision shows up in CI
   instead of in the field.

## What the contract promises you

- Every set in `input.sets` belongs to your algorithm; you never see another
  algorithm's sets, and you do not have to filter by `algorithm_id`.
- Sets arrive in no particular order. `members` inside a set is sorted by
  ascending throughput threshold.
- `input.budget_kbps()` resolves the transport estimate and the operator cap,
  including the "there is no estimate yet" case; algorithms that care about the
  difference read `bandwidth_estimate_kbps` directly.
- `input.stream_timeouts_since_last_decision` is evidence for _this_ decision
  only: the relay counts the streams it reset for delivery timeout and hands the
  count to the next decision, so it cannot go stale the way a flag does.

## What you owe the contract

- An answer for every set you were given, `Some(track)` or `None`. A set you
  leave out is not "leave it alone": the relay cannot wait forever for a
  decision that is never coming, so it forwards nothing and says so.
- Only tracks that are members of the set they are returned for.
- No blocking. `decide` runs on the connection's decision loop; anything that
  needs to wait belongs in the host, collected into `AbrInput` first.
