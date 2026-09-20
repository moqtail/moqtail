---
'moqtail-rs': minor
'relay': minor
---

Added SSTS (Sender Side Track Switching) support, gated behind `--enable-ssts`
because the setup option and message parameter it uses are provisional on an
unadopted draft. Ships the default allocation algorithm and a backpressure tier
selector; the algorithms live in the `moqtail-ssts` crate.
