---
'moqtail-rs': minor
'moqtail': minor
'relay': minor
'client': minor
'client-js': patch
---

Switch tracks with the SWITCH_FROM parameter

A SUBSCRIBE or REQUEST_UPDATE carrying SWITCH_FROM activates its own
subscription and suspends the one it names, in place of the SWITCH message,
which is gone. A hard switch stops the suspended subscription at the cutover; a
soft switch lets it drain to the group before the new subscription's start, so
the two are contiguous.

The fill filter types — AbsoluteStartFill, AbsoluteRangeFill and the new
RelativeStartFill — ask the publisher for the objects already published, which
arrive on a fill fetch stream: a unidirectional stream beginning with a
FETCH_HEADER that names the request which asked for the fill. FILL_PARAMETERS
overrides the subscriber priority and group order that stream runs at.

FETCH is a single message now. Fetch Type and Joining Fetch are gone, along with
INVALID_JOINING_REQUEST_ID, whose codepoint INVALID_SWITCH takes over.
