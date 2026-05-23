---
'relay': patch
---

Fix Joining Fetch range calculation per draft §9.16.2.

- **End location**: The relay now correctly sets End Location to `{LargestObject.Group, LargestObject.Object + 1}` (exclusive), so a Joining Fetch with `joining_start = 0` delivers all objects from the I-frame through the current LargestObject — previously the end was `{LargestObject.Group, 0}`, making the range empty for `joining_start = 0` and omitting the current partial group for any other value.
- **LargestObject in SubscribeOk**: When a subscriber arrives for an already-confirmed track, the relay now injects the track's current `LargestObject` position as a `MessageParameter::LargestObject` in the SubscribeOk, allowing the subscriber to decide whether to issue a Joining Fetch.
