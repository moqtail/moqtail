# moqtail-rs

## 0.19.0

### Minor Changes

- [#473](https://github.com/moqtail/moqtail/pull/473) [`94eb2d5`](https://github.com/moqtail/moqtail/commit/94eb2d59c8b25510af430d9ef103db235f83eb2a) Thanks [@zafergurel](https://github.com/zafergurel)! - qlog traces of control messages. Set `qlog_folder` in the relay config, or pass `--qlog-folder` to the client, to write one JSON-SEQ `.sqlog` file per connection with a `moqt:control_message_created`/`_parsed` event (decoded message plus raw bytes) for every control message, in the MoQT qlog event format. Unset, nothing is recorded. The per-message hex dumps at INFO level are removed. The relay logs to stdout only; it no longer writes a rolling `relay.log` into `log_folder`.

  In debug builds the relay also writes a QUIC packet-level qlog per connection (`*_quic.sqlog`: packets sent, received and lost, RTT and congestion window) into the same folder.

- [#474](https://github.com/moqtail/moqtail/pull/474) [`1409d03`](https://github.com/moqtail/moqtail/commit/1409d03bbe14090174d99f60a02ceec3aab92609) Thanks [@zafergurel](https://github.com/zafergurel)! - qlog traces now also record a `moqt:subgroup_header_*`/`moqt:fetch_header_*` event for every data stream opened or accepted (one per stream, not per object). `scripts/qlog-summary.py <folder>` prints each connection's peer SETUP, requests with their responses, data streams per track and parse errors.

- [#467](https://github.com/moqtail/moqtail/pull/467) [`bca5642`](https://github.com/moqtail/moqtail/commit/bca5642ba085d00f7f107972becfa6e85d6f5a77) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Relays can form a mesh. Set `hop_id` to declare this relay's Hop ID in SETUP and list other relays in `peers` to dial them. Namespaces announced or PUBLISHed on one relay are advertised to peers with HOP_PATH and ROUTE_COST (accumulating each sender's RELAY_COST, `relay_cost`), never back through a relay already on the path, and SUBSCRIBE/FETCH/TRACK_STATUS go to the one source the draft's path selection picks. `moqtail-rs` gains the HOP_PATH/ROUTE_COST message parameters and NAMESPACE's optional parameter block.

### Patch Changes

- [#472](https://github.com/moqtail/moqtail/pull/472) [`6520af0`](https://github.com/moqtail/moqtail/commit/6520af00de7d587fd3ee8471782acf0a36765170) Thanks [@DenizUgur](https://github.com/DenizUgur)! - fetch messages now use the priority requested by the FETCH request

- [#475](https://github.com/moqtail/moqtail/pull/475) [`9e8bf05`](https://github.com/moqtail/moqtail/commit/9e8bf05f99b6aeb27b07225fb1f5014aeb40f6ac) Thanks [@zafergurel](https://github.com/zafergurel)! - The relay names its qlog files by connection id alone (`server_conn<id>.sqlog`) and never overwrites an earlier trace: an id seen again after a restart gets a `_2`, `_3`, … suffix.

## 0.18.0

### Minor Changes

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - share each Object between a track's subscribers instead of cloning it for every one (`Object::serialize_subgroup_parts`)

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - write subgroup object payloads to subscribers without copying them (`SubgroupObject::serialize_parts`, `TransportSendStream::write_all_chunks`)

### Patch Changes

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - fix a data stream reader that could miss its wakeup and wait forever

- [#462](https://github.com/moqtail/moqtail/pull/462) [`a63df4e`](https://github.com/moqtail/moqtail/commit/a63df4e3aa1ae219bd2886bd3b888a64617d26be) Thanks [@zafergurel](https://github.com/zafergurel)! - parse incoming data streams without copying the receive buffer on every read

## 0.17.0

### Minor Changes

- [#444](https://github.com/moqtail/moqtail/pull/444) [`8c71f44`](https://github.com/moqtail/moqtail/commit/8c71f447385902e7e9d35f15cddb0f276fbc7f75) Thanks [@zafergurel](https://github.com/zafergurel)! - Serve FETCH from the relay's cache whenever the cache accounts for every Object of a group, and fetch upstream only what it does not. The relay now creates a track for a FETCH of a track it does not have yet, keeps a track for its cached Objects after its upstream goes away, records which Objects are known not to exist (from End of Group and End of Track statuses, gap properties, a group's last datagram or subgroup stream, and what an upstream FETCH response passes over), and lets concurrent FETCHes of the same range share one upstream FETCH. A PUBLISH_DONE now waits for a track's datagrams to go quiet. Both libraries parse End of Timed-Out Range markers and hand End of Range markers to the application (`RecvDataStream::next_item` in moqtail-rs, an `onEndOfRange` handler in moqtail). The test client can publish groups that already exist at startup, reset streams, drop datagrams, send End of Group and End of Track objects, and cut or mark its FETCH responses.

- [#459](https://github.com/moqtail/moqtail/pull/459) [`561588a`](https://github.com/moqtail/moqtail/commit/561588ae232fe7660bc69ee2812a98231d6481d0) Thanks [@zafergurel](https://github.com/zafergurel)! - Fix the LOC property ids: TIMESTAMP is 0x10 and VIDEO_FRAME_MARKING is 0x09, and AUDIO_CONFIG (0x0F) is added. Video frame marking now carries the RFC 9626 flag byte instead of a varint, so `addVideoFrameMarking` takes `isKey: boolean` and receivers check `independent` (`is_independent_frame()` in moqtail-rs). Timestamps are microseconds; meet and client-js publish them that way.

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

## 0.14.0

### Minor Changes

- [`0ca44e5`](https://github.com/moqtail/moqtail/commit/0ca44e59cf39ac97e73e465e80b64dba0302b2ba) Thanks [@zafergurel](https://github.com/zafergurel)! - Add raw QUIC transport support alongside WebTransport. `TransportConnection`/`TransportSendStream`/`TransportRecvStream` in moqtail-rs now wrap both wtransport and raw quinn behind one API, threaded through the relay and client stream handlers. The relay's QUIC listener demultiplexes WebTransport and raw-QUIC connections on the same UDP socket/port via ALPN, and the client connects over raw QUIC when given a `moqt://authority[/path]` server URL (carrying authority/path via CLIENT_SETUP parameters instead of HTTP CONNECT).

### Patch Changes

- [#208](https://github.com/moqtail/moqtail/pull/208) [`211ce94`](https://github.com/moqtail/moqtail/commit/211ce94bd1890cb4846d1805dc476e2b6702321b) Thanks [@zafergurel](https://github.com/zafergurel)! - Fix Key-Value-Pair Type fields being encoded/decoded as raw absolute values instead of deltas from the previous Type in the same list, per draft-ietf-moq-transport-16 (#1315). This broke interop with spec-compliant v16 peers: parameters such as FORWARD or SUBSCRIPTION_FILTER could be received as the wrong type (e.g. logged as "Unsupported parameter 48/81") by any peer that correctly delta-decodes. Affects Setup/Message Parameters and Object/Track Extension Headers (including the nested ImmutableExtensions list) in both the Rust (`moqtail-rs`) and TypeScript (`moqtail`) libraries.

- [#206](https://github.com/moqtail/moqtail/pull/206) [`9c236c9`](https://github.com/moqtail/moqtail/commit/9c236c9117578aaf1033ee1a8e7e52ad04086979) Thanks [@kerembkmz](https://github.com/kerembkmz)! - Normalize empty subgroup extension headers to null so [] serializes the same as no headers.

## 0.13.1

### Patch Changes

- [`50eeb39`](https://github.com/moqtail/moqtail/commit/50eeb3975d976618d7dcdf7c1b30f8ccabb3bdbd) Thanks [@zafergurel](https://github.com/zafergurel)! - Update draft version from 14 to 16 in Cargo.toml

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

## 0.9.0

### Minor Changes

- [#95](https://github.com/moqtail/moqtail/pull/95) [`5cd1a9e`](https://github.com/moqtail/moqtail/commit/5cd1a9ee3a04cb0a086c0873772d1ed8f85136d1) Thanks [@zafergurel](https://github.com/zafergurel)! - Add datagram support for publishing and subscribing to objects via QUIC datagrams

- [#92](https://github.com/moqtail/moqtail/pull/92) [`fa6c468`](https://github.com/moqtail/moqtail/commit/fa6c468ed5dc0dd8fb6a8ff2d07e063394cddeb5) Thanks [@DenizUgur](https://github.com/DenizUgur)! - update wtransport create

- [#96](https://github.com/moqtail/moqtail/pull/96) [`c94f626`](https://github.com/moqtail/moqtail/commit/c94f626cf320f18b6b042a6da971a6a2dc1108bd) Thanks [@fatih-alperen](https://github.com/fatih-alperen)! - Added support for TRACK_STATUS control messages

- [#91](https://github.com/moqtail/moqtail/pull/91) [`efe78fa`](https://github.com/moqtail/moqtail/commit/efe78fa367d667df72e54537202ae3d2695b8e6b) Thanks [@zafergurel](https://github.com/zafergurel)! - Fixes the control stream priority bug.

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

## 0.6.0

### Minor Changes

- [`e5121f2`](https://github.com/moqtail/moqtail/commit/e5121f26308b9d4219ee8e8b163742a8a32986da) Thanks [@acbegen](https://github.com/acbegen)! - Updates for Draft-14 compatibility

## 0.4.0

### Minor Changes

- [#58](https://github.com/streaming-university/moqtail/pull/58) [`7946290`](https://github.com/streaming-university/moqtail/commit/7946290b732367bac5bd2f81144c470f173c95b6) Thanks [@zafergurel](https://github.com/zafergurel)! - Handle MaxRequestId message

## 0.3.0

### Minor Changes

- [#51](https://github.com/streaming-university/moqtail/pull/51) [`59f2139`](https://github.com/streaming-university/moqtail/commit/59f213964436023f510bb1a3ba941c298f9904c5) Thanks [@DenizUgur](https://github.com/DenizUgur)! - Upgraded Version/Setup parameters to Draft-11

## 0.2.0

### Minor Changes

- [#44](https://github.com/streaming-university/moqtail/pull/44) [`a546e7b`](https://github.com/streaming-university/moqtail/commit/a546e7bcd260cc5cb6273504feea358d8c0886ba) Thanks [@zafergurel](https://github.com/zafergurel)! - Implement standalone fetch support for track retrieval.
