---
'moqtail-rs': patch
'moqtail': patch
---

Fix Key-Value-Pair Type fields being encoded/decoded as raw absolute values instead of deltas from the previous Type in the same list, per draft-ietf-moq-transport-16 (#1315). This broke interop with spec-compliant v16 peers: parameters such as FORWARD or SUBSCRIPTION_FILTER could be received as the wrong type (e.g. logged as "Unsupported parameter 48/81") by any peer that correctly delta-decodes. Affects Setup/Message Parameters and Object/Track Extension Headers (including the nested ImmutableExtensions list) in both the Rust (`moqtail-rs`) and TypeScript (`moqtail`) libraries.
