---
'moqtail-ssts': minor
---

backpressure: the subscriber can signal `U_min` as well as `rho`. A set's
`algorithm_params` now read as a positional layout: with one parameter it
is the probing risk tolerance as before (a trailing percentage, 1..=100);
with two or more, slot 0 is the minimum upshift stability threshold, a
count of media groups, and the trailing parameter is still the tolerance.
A signalled `U_min` outranks the recommended buffer-drainage horizon
`ceil(B / G)`, which stays the automatic default when the slot is zero or
absent; the written five still answers when neither clock is known. A new
operator knob, `backpressure.umin`, replaces the derived horizon for subscribers
that signal none.
