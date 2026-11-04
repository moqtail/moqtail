---
'moqtail': minor
---

Extensions can manage switching sets: `ExtensionDefinition` gains `switchingSetSource`, loaded for each media switching set with the probed source as its `child` (`SwitchingSetOptions`), and `negotiated`, which checks the peer's SETUP. Two listed extensions with a `switchingSetSource` throw `UNSUPPORTED_SOURCE` from the player constructor.
