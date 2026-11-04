---
'moqtail': minor
---

Add object ordering to the client. An `ordering` option on `MOQtailClientOptions`, `SubscribeOptions`, and `FetchOptions` releases a request's objects in location order, whether they arrive on one stream per group, one per object, a fill fetch stream, or datagrams. It reads Prior Group ID Gap and Prior Object ID Gap, waits within a `window` for a missing object, and reports what it stopped waiting for with the new `orderinggap` event. Without `ordering`, objects pass through unordered as before.
