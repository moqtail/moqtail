---
'moqtail': minor
'client-js': minor
---

Breaking: `CMSFCatalog`, `CMSF`, and the rest of the old catalog model are removed from `moqtail`. Read catalogs through the player's `catalog` event, and type a catalog you publish as `CatalogJson` from `moqtail/player`. client-js now plays through `<moqtail-video>` inside Video.js v10, and its publisher writes a catalog for the current MSF draft.
