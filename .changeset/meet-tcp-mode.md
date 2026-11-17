---
'meet': minor
---

Meet can run over TCP. Settings has a Transport choice: Auto (the default, falling back to WebSocket when WebTransport can't connect), WebTransport (UDP) or WebSocket (TCP). It is saved in the browser, and `appsettings.js` can set a default with `transport`. The call header shows which transport the call runs on.
