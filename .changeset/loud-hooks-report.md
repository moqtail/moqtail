---
'moqtail': patch
---

Report request-stream traffic through `onMessageSent` and `onMessageReceived`: only the control stream fired them, so SUBSCRIBE, PUBLISH, FETCH, the namespace messages and their replies went unreported
