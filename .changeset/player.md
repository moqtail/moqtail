---
'moqtail': minor
---

Add `moqtail/player`, an MSF/CMSF player built from elements, and the `<moqtail-video>` custom element in `moqtail/player/component`.

Breaking: the old catalog model (`CMSFCatalog`, `CMSF`, …) is removed. Read catalogs through the player's `catalog` event and type published ones as `CatalogJson`.

- `MOQtailPlayer` builds a source → transform → sink chain, shares one reconnecting client across players on a page, and reports `statechange`, `error` and `warning`.
- An element runtime (`defineElement`, source/transform/sink base classes, pads with backpressure) for custom elements.
- Built-in catalog source with delta updates, a CMAF source with seeking, URL ranges and rendition switching, and an MSE sink.
- Media and event timelines, per-request object ordering, pluggable buffer controllers, and the `SSTS` extension.
- `parseMsfUrl` and `formatMsfUrl` for `moqt://...#msf:` URLs.
