---
'moqtail': patch
---

`client.switch` no longer sends FORWARD on its SWITCH_FROM SUBSCRIBE, which relays reject: the relay controls forwarding during a switch. `SwitchOptions.newSubscribeOptions` no longer takes `forward`. `onDrainDecision` returning false now ends the drain at once, releasing the new track's held objects instead of waiting for PUBLISH_DONE or the drain timeout. An ordered SUBSCRIBE with `FillParameters` waits for its fill fetch stream before reporting objects as missing.
