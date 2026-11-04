---
'moqtail': minor
---

`client.switch` returns the new request's full `SubscribeResult`, with its `subscribeOk`, the original subscription's `stream`, and its `done`. `subscribeUpdate` takes `priority` and `forward` as optional and only sends SUBSCRIBER_PRIORITY and FORWARD when they're given.
