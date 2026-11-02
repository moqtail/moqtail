---
'moqtail': minor
'moqtail-rs': minor
'meet': patch
'client-js': patch
---

Fix the LOC property ids: TIMESTAMP is 0x10 and VIDEO_FRAME_MARKING is 0x09, and AUDIO_CONFIG (0x0F) is added. Video frame marking now carries the RFC 9626 flag byte instead of a varint, so `addVideoFrameMarking` takes `isKey: boolean` and receivers check `independent` (`is_independent_frame()` in moqtail-rs). Timestamps are microseconds; meet and client-js publish them that way.
