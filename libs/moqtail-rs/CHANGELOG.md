# moqtail-rs

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
