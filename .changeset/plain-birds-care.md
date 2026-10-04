---
'moqtail': minor
'moqtail-rs': minor
'moqtail-ssts': minor
'relay': minor
---

Added SSTS (Sender Side Track Switching) support, gated behind `--ssts-enable` since the setup option and the message parameter it uses are provisional on an unadopted draft. Also implements the default ABR algortihm and a backpressure based experimental algorithm. Algorithms live in the `moqtail-ssts` crate.
