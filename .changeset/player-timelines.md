---
'moqtail': minor
---

Add media and event timelines to the player. Sources resolve media time and wallclock seeks and URL ranges through a media timeline track or a `template`, honouring `accessibleWindowTime`. Opted-in event timelines (`eventTimelines`) deliver each record once through the typed `timelineevent` event and an `eventtimeline` source pad, with the built-in `SapTimeline` for CMSF SAP type timelines. `TimelineEvent` and `TimelineRecord` are exported from `moqtail/player`.
