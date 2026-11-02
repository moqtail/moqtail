---
'moqtail': minor
'client': patch
'client-js': patch
---

Add `moqtail/player` with MSF URL parsing: `parseMsfUrl` turns a `moqt://...#msf:` URL into its namespace, track name, ranges and parameters, and `formatMsfUrl` builds one. `moqt://` is matched case-insensitively in `MoqtUrl.parse`, `resolveTransportUrl` and the Rust client. client-js share links now always name the `catalog` track.
