---
'relay': patch
'moqtail-rs': patch
---

Fixes found by the moq-web interop tests:

- A fill opened by REQUEST_UPDATE now carries the REQUEST_UPDATE's Request ID in its FETCH_HEADER, not the subscription's, so the subscriber can match it.
- A message on a request stream that does not parse, such as one carrying an unknown parameter, now closes the session with PROTOCOL_VIOLATION. It used to be dropped, or taken as the peer cancelling the request.
- A padding stream is read to its end and discarded instead of being rejected as a malformed data stream.
- A subscriber's NEW_GROUP_REQUEST now reaches the publisher: on the publisher's PUBLISH stream or the relay's upstream SUBSCRIBE as a REQUEST_UPDATE, or with the first upstream SUBSCRIBE. It is passed on only when the track has DYNAMIC_GROUPS, the value is 0 or past the largest group, and no larger request is still waiting for its group.
