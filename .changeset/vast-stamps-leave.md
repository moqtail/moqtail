---
'client': minor
'client-js': minor
'meet': minor
'relay': minor
'moqtail-rs': minor
'moqtail': minor
'moqtail-ssts': minor
---

- Changes related to draft-18 compatibility
- Added SSTS (Sender Side Track Switching) support, gated behind `--ssts-enable` since the setup option and the message parameter it uses are provisional on an unadopted draft. Also implements the default ABR algorithm and a backpressure based experimental algorithm. Algorithms live in the `moqtail-ssts` crate.
- Rename warp to cmsf and make catalog version string
- Refactor enums to use tryFrom for cleaner API
- Keep a publication alive when the peer moves its Forward State, and report the move through onForwardStateChange
- Tell onPeerPublishDone which request the PUBLISH_DONE ended
- Fix(moqtail-ts): stop TerminationCode.tryFrom throwing on five valid enum values
- Client: add a subscribe-namespace command, receive objects for the tracks SUBSCRIBE_TRACKS is handed, cancel a prefix subscription by resetting its request stream, and report REQUEST_ERROR with its code and reason
- Handle malformed FETCH track on end client
- Helper function to validate subgroup priority in FETCH
- Check prior id gaps for malformed track
- Relay: reset downstream FETCH on malformed upstream track
- Relay: handle objects received before FETCH_OK
- Relay: report stream closed for empty FETCHes
