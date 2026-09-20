---
'moqtail': minor
---

Add the provisional SSTS model surface: the `SstsAlgorithms` SETUP option
(0x09, draft-wilaw-moq-moqt-ssts §3.1) and the `SwitchingSetAssignment`
message parameter (0x41, §5). Both codepoints come from an unadopted
draft, so they are recorded as `local_extensions` in the shared
`dev/conformance/draft18` fixture rather than the adopted draft-18
registry, and are documented as provisional at every use site.
