---
'moqtail-rs': patch
'relay': patch
'moqtail': patch
---

Encode the TRACK_NAMESPACE_PREFIX parameter in REQUEST_UPDATE as a bare Track Namespace, with the field count right after the parameter type. It was written with a length in front, so prefix updates from other implementations failed to parse at the relay.
