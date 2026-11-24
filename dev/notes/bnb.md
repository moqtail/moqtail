# BNB (budget-negotiated backpressure)

BNB was an experimental SSTS tier controller on the unused private id `0xff03`.
It gave each switching set its own rung and treated the subscriber's negotiated
reorder buffer as the deadline a group had to beat.

It never produced a result worth keeping. The group-span measurement it depended
on is noisy with concurrent subscribers and re-sent streams, and the controller
was parked unregistered before the public demo. It has been removed rather than
shipped, and `0xff03` is intentionally left unused.

The surviving controllers are `asd`, `asd_v11`, the `wasd` family (the
backpressure experiment) and the testbed oracle.
