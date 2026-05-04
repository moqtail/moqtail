# meet

## 0.2.0

### Minor Changes

- [#145](https://github.com/moqtail/moqtail/pull/145) [`1b855cf`](https://github.com/moqtail/moqtail/commit/1b855cfece77cbade63f8263f485b8b5c7839134) Thanks [@zafergurel](https://github.com/zafergurel)! - Implement MOQ Transport draft-16 compliance across all packages.
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
