# meet

## 0.3.0

### Minor Changes

- [#440](https://github.com/moqtail/moqtail/pull/440) [`55eaa12`](https://github.com/moqtail/moqtail/commit/55eaa1234fd394c13ae0c2ea0f50e60e66605578) Thanks [@zafergurel](https://github.com/zafergurel)! - Add a settings pane for the relay address, timetravel-style call controls with a leave button, and a pane that lists the control and signalling messages sent and received with timestamps, filterable by kind and type, and a Data tab with per-track counters for streams, groups, subgroups, objects, bytes, rates, interarrival time, jitter and latency in each direction

- [#440](https://github.com/moqtail/moqtail/pull/440) [`55eaa12`](https://github.com/moqtail/moqtail/commit/55eaa1234fd394c13ae0c2ea0f50e60e66605578) Thanks [@zafergurel](https://github.com/zafergurel)! - Ask for a room name when joining. Each room gets its own namespace, `moqtail/demo/<room>`, with its own signalling track and its users' namespaces under it, so only people in the same room see each other

### Patch Changes

- [#440](https://github.com/moqtail/moqtail/pull/440) [`55eaa12`](https://github.com/moqtail/moqtail/commit/55eaa1234fd394c13ae0c2ea0f50e60e66605578) Thanks [@zafergurel](https://github.com/zafergurel)! - Stop sending audio objects while the mic is muted: a muted track still yields silence, which was encoded and sent

## 0.2.0

### Minor Changes

- [#391](https://github.com/moqtail/moqtail/pull/391) [`fbec509`](https://github.com/moqtail/moqtail/commit/fbec5094b0be8fbb1431801093d091444c331a53) - - Changes related to draft-18 compatibility
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

## 0.1.0

### Minor Changes

- [#145](https://github.com/moqtail/moqtail/pull/145) [`1b855cf`](https://github.com/moqtail/moqtail/commit/1b855cfece77cbade63f8263f485b8b5c7839134) Thanks [@zafergurel](https://github.com/zafergurel)! - Initial version of meet video conferencing demo app
