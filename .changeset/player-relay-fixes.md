---
'moqtail': minor
---

Player fixes from the first relay runs:

- Fast switching follows draft-ugur-moq-fast-switching: group IDs on the set's alignment unit, starting at 0, and a switch into another GoP ladder enters at the track with the nearest switch point, then moves up the ladder. Seek estimates land on each track's group grid.
- A live MSE sink keeps everything ahead of `currentTime`; `forwardCap` only applies to VOD. `backBuffer` defaults to 90 s.
- The player no longer reports `stalled` while the video element waits for a seek or before it first plays, nor `paused` right before `ended`. `MediaSinkElement` gains `seeking` and `ended`.
- `SstsSource` refuses a session that didn't negotiate SSTS, and fails only when every track of the set failed to load.
- The client leaves no unhandled rejections when the session closes, and doesn't log a closing session as an error.
