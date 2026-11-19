---
'moqtail': minor
---

Pads carry a `Packet` (the track's rendition id, location, object properties and payload) instead of a `MoqtObject`: custom elements read `item.packet` in `receive()` and write `{ type: 'packet', packet }`.
