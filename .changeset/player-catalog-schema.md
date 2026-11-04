---
'moqtail': minor
---

`moqtail/player` exports the `Catalog`, `CatalogJson`, `CatalogTrack`, `InitData`, and `MediaTimelineTemplate` types of the MSF and CMSF catalog, derived from Valibot schemas. `CatalogTrack` follows the draft field names (`maxGroupDuration`, not `maxGrpDuration`) and types `template`. Valibot is a new dependency, used only by the player's catalog element, so `import 'moqtail'` doesn't include it.
