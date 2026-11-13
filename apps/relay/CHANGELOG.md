# relay

## 0.19.0

### Minor Changes

- [#473](https://github.com/moqtail/moqtail/pull/473) [`94eb2d5`](https://github.com/moqtail/moqtail/commit/94eb2d59c8b25510af430d9ef103db235f83eb2a) Thanks [@zafergurel](https://github.com/zafergurel)! - qlog traces of control messages. Set `qlog_folder` in the relay config, or pass `--qlog-folder` to the client, to write one JSON-SEQ `.sqlog` file per connection with a `moqt:control_message_created`/`_parsed` event (decoded message plus raw bytes) for every control message, in the MoQT qlog event format. Unset, nothing is recorded. The per-message hex dumps at INFO level are removed. The relay logs to stdout only; it no longer writes a rolling `relay.log` into `log_folder`.

  In debug builds the relay also writes a QUIC packet-level qlog per connection (`*_quic.sqlog`: packets sent, received and lost, RTT and congestion window) into the same folder.

- [#474](https://github.com/moqtail/moqtail/pull/474) [`1409d03`](https://github.com/moqtail/moqtail/commit/1409d03bbe14090174d99f60a02ceec3aab92609) Thanks [@zafergurel](https://github.com/zafergurel)! - qlog traces now also record a `moqt:subgroup_header_*`/`moqt:fetch_header_*` event for every data stream opened or accepted (one per stream, not per object). `scripts/qlog-summary.py <folder>` prints each connection's peer SETUP, requests with their responses, data streams per track and parse errors.

- [#467](https://github.com/moqtail/moqtail/pull/467) [`bca5642`](https://github.com/moqtail/moqtail/commit/bca5642ba085d00f7f107972becfa6e85d6f5a77) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Relays can form a mesh. Set `hop_id` to declare this relay's Hop ID in SETUP and list other relays in `peers` to dial them. Namespaces announced or PUBLISHed on one relay are advertised to peers with HOP_PATH and ROUTE_COST (accumulating each sender's RELAY_COST, `relay_cost`), never back through a relay already on the path, and SUBSCRIBE/FETCH/TRACK_STATUS go to the one source the draft's path selection picks. `moqtail-rs` gains the HOP_PATH/ROUTE_COST message parameters and NAMESPACE's optional parameter block.

- [#466](https://github.com/moqtail/moqtail/pull/466) [`44ff0df`](https://github.com/moqtail/moqtail/commit/44ff0df5a3936139653491a9554dc2c21f13565a) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Move the relay's settings into a YAML file passed with `--config` (see `apps/relay/relay.example.yaml`). The command line keeps only `--config`, `--bind`, `--cert-file` and `--key-file`, which override the file. `--host`/`--port` are now `--bind <ip:port>`, and durations take units (`10s`, `500ms`, `30m`), so the `_secs`/`_ms`/`_minutes` suffixes are gone from the keys.

### Patch Changes

- [#472](https://github.com/moqtail/moqtail/pull/472) [`6520af0`](https://github.com/moqtail/moqtail/commit/6520af00de7d587fd3ee8471782acf0a36765170) Thanks [@DenizUgur](https://github.com/DenizUgur)! - fetch messages now use the priority requested by the FETCH request

- [#475](https://github.com/moqtail/moqtail/pull/475) [`9e8bf05`](https://github.com/moqtail/moqtail/commit/9e8bf05f99b6aeb27b07225fb1f5014aeb40f6ac) Thanks [@zafergurel](https://github.com/zafergurel)! - The relay names its qlog files by connection id alone (`server_conn<id>.sqlog`) and never overwrites an earlier trace: an id seen again after a restart gets a `_2`, `_3`, … suffix.

## 0.18.0

### Minor Changes

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - ask for 8 MiB UDP socket buffers by default (`--socket-buffer-kib`, 0 keeps the kernel default), so a fan-out burst no longer overflows the socket

### Patch Changes

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - fix a data stream reader that could miss its wakeup and wait forever

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - visit only the tracks a closing publisher published, not every track

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - on disconnect, visit only the tracks the client ever subscribed to

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - stop holding the tracks map while a closed connection is removed from its tracks

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - count a subgroup object as the largest before subscribers get it, so one joining then misses nothing

- [#445](https://github.com/moqtail/moqtail/pull/445) [`cab350a`](https://github.com/moqtail/moqtail/commit/cab350a3f2192b130564b23d3867cafb155954b9) Thanks [@sharmafb](https://github.com/sharmafb)! - cancel upstream requests on malformed track

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - drop the per-track lock; tracks' fields already lock themselves

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - keep the track manager's maps private

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - group the relay's server modules into session, track, subscription and logging folders

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - keep downstream and upstream requests in one table each

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - share each Object between a track's subscribers instead of cloning it for every one (`Object::serialize_subgroup_parts`)

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - fan out under one subscriber lock and read the default publisher priority without one

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - split the FETCH handler's upstream request and stream delivery into their own modules

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - split the SUBSCRIBE handler's upstream exchange and REQUEST_UPDATE into their own modules

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - split the subscription's delivery, stream writing and switching into their own modules

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - split the track's ingest, publisher-stream waiting and upstream requests into their own modules

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - move the SSTS forward gate into its own type

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - Skip the rest of a group for a subscriber whose queue is too old or too large, set by `--delivery-timeout-ms` and `--max-subscriber-queue-bytes`.

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - move subscription state into its own module

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - reach a track's status and properties through its methods

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - write subgroup object payloads to subscribers without copying them (`SubgroupObject::serialize_parts`, `TransportSendStream::write_all_chunks`)

## 0.17.0

### Minor Changes

- [#444](https://github.com/moqtail/moqtail/pull/444) [`8c71f44`](https://github.com/moqtail/moqtail/commit/8c71f447385902e7e9d35f15cddb0f276fbc7f75) Thanks [@zafergurel](https://github.com/zafergurel)! - Serve FETCH from the relay's cache whenever the cache accounts for every Object of a group, and fetch upstream only what it does not. The relay now creates a track for a FETCH of a track it does not have yet, keeps a track for its cached Objects after its upstream goes away, records which Objects are known not to exist (from End of Group and End of Track statuses, gap properties, a group's last datagram or subgroup stream, and what an upstream FETCH response passes over), and lets concurrent FETCHes of the same range share one upstream FETCH. A PUBLISH_DONE now waits for a track's datagrams to go quiet. Both libraries parse End of Timed-Out Range markers and hand End of Range markers to the application (`RecvDataStream::next_item` in moqtail-rs, an `onEndOfRange` handler in moqtail). The test client can publish groups that already exist at startup, reset streams, drop datagrams, send End of Group and End of Track objects, and cut or mark its FETCH responses.

## 0.16.0

### Minor Changes

- [#386](https://github.com/moqtail/moqtail/pull/386) [`d642d33`](https://github.com/moqtail/moqtail/commit/d642d335d1f0b0bc4d2020dc47416eaeb2b4a550) Thanks [@zafergurel](https://github.com/zafergurel)! - Switch tracks with the SWITCH_FROM parameter

  A SUBSCRIBE or REQUEST_UPDATE carrying SWITCH_FROM activates its own
  subscription and suspends the one it names, in place of the SWITCH message,
  which is gone. A hard switch stops the suspended subscription at the cutover; a
  soft switch lets it drain to the group before the new subscription's start, so
  the two are contiguous.

  The fill filter types — AbsoluteStartFill, AbsoluteRangeFill and the new
  RelativeStartFill — ask the publisher for the objects already published, which
  arrive on a fill fetch stream: a unidirectional stream beginning with a
  FETCH_HEADER that names the request which asked for the fill. FILL_PARAMETERS
  overrides the subscriber priority and group order that stream runs at.

  FETCH is a single message now. Fetch Type and Joining Fetch are gone, along with
  INVALID_JOINING_REQUEST_ID, whose codepoint INVALID_SWITCH takes over.

### Patch Changes

- [#399](https://github.com/moqtail/moqtail/pull/399) [`c0621f3`](https://github.com/moqtail/moqtail/commit/c0621f306eb7c1511c95ba9c09dbb68dc4da441f) Thanks [@burak-kara](https://github.com/burak-kara)! - Fix relay deadlocks between closing sessions, track removal, and track lookups by alias or by publisher

- [#397](https://github.com/moqtail/moqtail/pull/397) [`862fc75`](https://github.com/moqtail/moqtail/commit/862fc75fda090186ac0d3ac33183ed8153125567) Thanks [@sharmafb](https://github.com/sharmafb)! - terminate downstream subscriptions on malformed upstream track

## 0.15.0

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

## 0.14.1

### Patch Changes

- [#219](https://github.com/moqtail/moqtail/pull/219) [`78bfab7`](https://github.com/moqtail/moqtail/commit/78bfab7eb51847da64c473e8f3ff2b8165d1c198) Thanks [@zafergurel](https://github.com/zafergurel)! - Fixes missing header bug. Thanks to [kkroo](https://github.com/kkroo)

## 0.14.0

### Minor Changes

- [`0ca44e5`](https://github.com/moqtail/moqtail/commit/0ca44e59cf39ac97e73e465e80b64dba0302b2ba) Thanks [@zafergurel](https://github.com/zafergurel)! - Add raw QUIC transport support alongside WebTransport. `TransportConnection`/`TransportSendStream`/`TransportRecvStream` in moqtail-rs now wrap both wtransport and raw quinn behind one API, threaded through the relay and client stream handlers. The relay's QUIC listener demultiplexes WebTransport and raw-QUIC connections on the same UDP socket/port via ALPN, and the client connects over raw QUIC when given a `moqt://authority[/path]` server URL (carrying authority/path via CLIENT_SETUP parameters instead of HTTP CONNECT).

### Patch Changes

- [#201](https://github.com/moqtail/moqtail/pull/201) [`41d6ff4`](https://github.com/moqtail/moqtail/commit/41d6ff4e054a485640a099f8bd94b3866e0b0dcb) Thanks [@zafergurel](https://github.com/zafergurel)! - Fix silent object drops for subscribers joining a track mid-subgroup. New subscribers had no open QUIC send stream for an in-progress subgroup, so objects with no header info were silently discarded. The relay now caches the original subgroup header when the first object of each subgroup arrives and uses it to open a send stream for late joiners. The cache entry is evicted when the publisher unistream closes.

- [#201](https://github.com/moqtail/moqtail/pull/201) [`41d6ff4`](https://github.com/moqtail/moqtail/commit/41d6ff4e054a485640a099f8bd94b3866e0b0dcb) Thanks [@zafergurel](https://github.com/zafergurel)! - When a subscription's `forward` parameter transitions from false to true via REQUEST_UPDATE mid-group, the relay now immediately opens a QUIC send stream for the subscriber using the subgroup header that arrived while forwarding was paused. Previously, if no new group boundary arrived after the toggle, the subscriber would miss the remainder of the current subgroup.

## 0.13.4

### Patch Changes

- [#199](https://github.com/moqtail/moqtail/pull/199) [`1c209c5`](https://github.com/moqtail/moqtail/commit/1c209c5bd1138c6ed4153d818d3d9a7f64278721) Thanks [@zafergurel](https://github.com/zafergurel)! - Fix relay not sending FETCH_OK for non-empty fetch ranges. The relay previously only sent FETCH_OK when the requested range was empty; for any fetch that returned objects the control-stream response was omitted, causing subscribers to block indefinitely waiting for it.

## 0.13.3

### Patch Changes

- [#197](https://github.com/moqtail/moqtail/pull/197) [`0ddb35a`](https://github.com/moqtail/moqtail/commit/0ddb35a1215877bda60abc3455d183c23fe4d95b) Thanks [@zafergurel](https://github.com/zafergurel)! - Fix Joining Fetch range calculation per draft §9.16.2.
  - **End location**: The relay now correctly sets End Location to `{LargestObject.Group, LargestObject.Object + 1}` (exclusive), so a Joining Fetch with `joining_start = 0` delivers all objects from the I-frame through the current LargestObject — previously the end was `{LargestObject.Group, 0}`, making the range empty for `joining_start = 0` and omitting the current partial group for any other value.
  - **LargestObject in SubscribeOk**: When a subscriber arrives for an already-confirmed track, the relay now injects the track's current `LargestObject` position as a `MessageParameter::LargestObject` in the SubscribeOk, allowing the subscriber to decide whether to issue a Joining Fetch.

## 0.13.2

### Patch Changes

- [#193](https://github.com/moqtail/moqtail/pull/193) [`0d2c5e7`](https://github.com/moqtail/moqtail/commit/0d2c5e7f5c4b726ad0612ec734163898e04d321a) Thanks [@sharmafb](https://github.com/sharmafb)! - Send fetches upstream for cache misses

- [#179](https://github.com/moqtail/moqtail/pull/179) [`7583b5b`](https://github.com/moqtail/moqtail/commit/7583b5bca1e34dd1b321271b5157eff147918a1f) Thanks [@davemevans](https://github.com/davemevans)! - chore: add instructions for Firefox testing using private CA

## 0.13.0

### Minor Changes

- [#145](https://github.com/moqtail/moqtail/pull/145) [`1b855cf`](https://github.com/moqtail/moqtail/commit/1b855cfece77cbade63f8263f485b8b5c7839134) Thanks [@zafergurel](https://github.com/zafergurel), [@fatih-alperen](https://github.com/fatih-alperen), [@ctllmp](https://github.com/ctllmp), [@beyzademirr](https://github.com/beyzademirr)! - Implement MOQ Transport draft-16 compliance across all packages.
  - New ALPN-based session setup flow (ClientSetup / ServerSetup)
  - Replaced VersionParameter with MessageParameter; added typed parameters: DeliveryTimeout, Expires, Forward, GroupOrder, LargestObject, NewGroupRequest, SubscriberPriority, SubscriptionFilter
  - Implemented TrackExtension and ObjectExtension; updated Publish, Subscribe, Fetch, PublishOk, SubscribeOk, FetchOk messages
  - Unified datagram wire format (Datagram replaces DatagramObject and DatagramStatus)
  - Implemented SubgroupHeader per draft-16 section 10.4.2 with three ID encoding modes
  - Unified OK responses: REQUEST_OK replaces PublishNamespaceOk, SubscribeNamespaceOk, TrackStatusOk
  - Unified error responses: REQUEST_ERROR replaces FetchError, PublishError, SubscribeError, SubscribeNamespaceError, PublishNamespaceError, TrackStatusError
  - SUBSCRIBE_UPDATE renamed to REQUEST_UPDATE; update propagation added for all message types
  - Unified request ID registry mapping request_id to handler type for correct response routing
  - Bitmask-based FetchObject serialization with delta encoding for sequential objects per draft-16 section 10.4.4
  - SUBSCRIBE_NAMESPACE uses a dedicated request stream; added Namespace and NamespaceDone messages
  - Relay implements draft-16 scheduling algorithm based on combined subscriber and publisher priorities
  - Removed synthetic Subscribe hack for Publish-based subscriptions
  - Renamed request_id field to max_request_id
  - Added client-js browser subscriber app and meet video conferencing demo app

## 0.12.0

### Minor Changes

- [#137](https://github.com/moqtail/moqtail/pull/137) [`19f19e7`](https://github.com/moqtail/moqtail/commit/19f19e71a1117d90be5d68c839adeb2b02cbc518) Thanks [@fatih-alperen](https://github.com/fatih-alperen)! - Fixed request_id errors and track_id errors.

- [#137](https://github.com/moqtail/moqtail/pull/137) [`19f19e7`](https://github.com/moqtail/moqtail/commit/19f19e71a1117d90be5d68c839adeb2b02cbc518) Thanks [@fatih-alperen](https://github.com/fatih-alperen)! - Added old publish tracking and matching for subscribe_namespace

## 0.11.1

### Patch Changes

- [`c16fab7`](https://github.com/moqtail/moqtail/commit/c16fab77395de3ccd99c25b43ed6fd2754129d70) Thanks [@zafergurel](https://github.com/zafergurel)! - feat: add datagram draft-14 support, remove deprecated AkamaiOffset, update package description
  - feat(moqtail-rs, moqtail-ts): Add datagram draft-14 compatibility across both
    the Rust and TypeScript libraries. Updates datagram object parsing, datagram
    status handling, object model, and constants in both libs; also adjusts the
    relay's track handling and the TypeScript client/datagram stream accordingly.
  - refactor(moqtail-ts): Remove the deprecated AkamaiOffset utility class from
    the TypeScript library. ClockNormalizer is its replacement. Cleans up the
    export index and updates the README to reflect the removal.
  - chore(moqtail-rs): Update the moqtail-rs crate description in Cargo.toml.

## 0.11.0

### Minor Changes

- [#104](https://github.com/moqtail/moqtail/pull/104) [`a08c438`](https://github.com/moqtail/moqtail/commit/a08c4380f7a0abd25fcfa424ce3fbb5d90b4a977) Thanks [@zafergurel](https://github.com/zafergurel)! - Implements SWITCH message

- [#106](https://github.com/moqtail/moqtail/pull/106) [`3763a1a`](https://github.com/moqtail/moqtail/commit/3763a1a21214d5b262e39abb45c29dc0c6484c4a) Thanks [@zafergurel](https://github.com/zafergurel)! - SUBSCRIBE_NAMESPACE implementation

## 0.10.0

### Minor Changes

- [#101](https://github.com/moqtail/moqtail/pull/101) [`9e7e57e`](https://github.com/moqtail/moqtail/commit/9e7e57e8ce0a82c374608f1d388eed138143e9b3) Thanks [@zafergurel](https://github.com/zafergurel)! - Support for fetch_cancel message

## 0.9.0

### Minor Changes

- [#95](https://github.com/moqtail/moqtail/pull/95) [`5cd1a9e`](https://github.com/moqtail/moqtail/commit/5cd1a9ee3a04cb0a086c0873772d1ed8f85136d1) Thanks [@zafergurel](https://github.com/zafergurel)! - Add datagram support for publishing and subscribing to objects via QUIC datagrams

- [#92](https://github.com/moqtail/moqtail/pull/92) [`fa6c468`](https://github.com/moqtail/moqtail/commit/fa6c468ed5dc0dd8fb6a8ff2d07e063394cddeb5) Thanks [@DenizUgur](https://github.com/DenizUgur)! - update wtransport create

- [#76](https://github.com/moqtail/moqtail/pull/76) [`9580a11`](https://github.com/moqtail/moqtail/commit/9580a117eecae92cf2b75dcba10b08b6f8674f90) Thanks [@zafergurel](https://github.com/zafergurel)! - fixes forward location handling

### Patch Changes

- [#96](https://github.com/moqtail/moqtail/pull/96) [`c94f626`](https://github.com/moqtail/moqtail/commit/c94f626cf320f18b6b042a6da971a6a2dc1108bd) Thanks [@fatih-alperen](https://github.com/fatih-alperen)! - Added support for TRACK_STATUS control messages

## 0.8.0

### Minor Changes

- [#74](https://github.com/moqtail/moqtail/pull/74) [`aa4ff01`](https://github.com/moqtail/moqtail/commit/aa4ff01d9c642de9f0d3fb5fdaaeb12d29abc8ee) Thanks [@zafergurel](https://github.com/zafergurel)! - Add auth token parameter as a setup parameter

## 0.7.1

### Patch Changes

- [#72](https://github.com/moqtail/moqtail/pull/72) [`af87c49`](https://github.com/moqtail/moqtail/commit/af87c49ee21cf255a0d47e218e793f58aa5ecadb) Thanks [@zafergurel](https://github.com/zafergurel)! - improved subscribe update handling

## 0.7.0

### Minor Changes

- [#69](https://github.com/moqtail/moqtail/pull/69) [`a22396a`](https://github.com/moqtail/moqtail/commit/a22396a9e891b2424a14b2ad18682473f890093d) Thanks [@acbegen](https://github.com/acbegen)! - The object id parsing in moqtail-rs is fixed.
  The relay code is refactored.

## 0.5.0

### Minor Changes

- [`3b5accf`](https://github.com/moqtail/moqtail/commit/3b5accf65d01bf264db64aaecd7a6215adc5ab4a) Thanks [@zafergurel](https://github.com/zafergurel)! - Updates for Draft-14 compatibility

## 0.4.0

### Minor Changes

- [#58](https://github.com/streaming-university/moqtail/pull/58) [`7946290`](https://github.com/streaming-university/moqtail/commit/7946290b732367bac5bd2f81144c470f173c95b6) Thanks [@zafergurel](https://github.com/zafergurel)! - Handle MaxRequestId message

- [#53](https://github.com/streaming-university/moqtail/pull/53) [`a5dfb19`](https://github.com/streaming-university/moqtail/commit/a5dfb196c1aab46f0183e5c70fd7193953dfd108) Thanks [@zafergurel](https://github.com/zafergurel)! - Implements a cache eviction policy by using a cache grow ratio before evicting.
