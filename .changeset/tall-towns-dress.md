---
'relay': patch
'moqtail-rs': patch
'moqtail': patch
---

feat: add datagram draft-14 support, remove deprecated AkamaiOffset, update package description

- feat(moqtail-rs, moqtail-ts): Add datagram draft-14 compatibility across both
  the Rust and TypeScript libraries. Updates datagram object parsing, datagram
  status handling, object model, and constants in both libs; also adjusts the
  relay's track handling and the TypeScript client/datagram stream accordingly.

- refactor(moqtail-ts): Remove the deprecated AkamaiOffset utility class from
  the TypeScript library. ClockNormalizer is its replacement. Cleans up the
  export index and updates the README to reflect the removal.

- chore(moqtail-rs): Update the moqtail-rs crate description in Cargo.toml.
