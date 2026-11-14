# Vendored crates

## wtransport

[wtransport](https://github.com/BiagioFesta/wtransport) 0.7.2 (MIT OR Apache-2.0),
patched into the workspace through `[patch.crates-io]` in the root `Cargo.toml`.

The one change is `Connection::close_session(error_code, reason)`. `Connection::close`
closes only the QUIC connection, which a browser reports as "Connection lost" with
code 0. `close_session` first sends a CLOSE_WEBTRANSPORT_SESSION capsule on the
session's CONNECT stream, waits up to 2 s for it to be acknowledged, and then closes.

Drop this copy once upstream wtransport can send that capsule
([BiagioFesta/wtransport#337](https://github.com/BiagioFesta/wtransport/pull/337)).
