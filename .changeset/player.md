---
'moqtail': minor
---

Add `moqtail/player`, an MSF/CMSF player built from elements, and the `<moqtail-video>` custom element in `moqtail/player/component`.

Breaking: the old catalog model (`CMSFCatalog`, `CMSF`, …) is removed. Read catalogs through the player's `catalog` event and type published ones as `CatalogJson`.

- `MOQtailPlayer` builds a source → transform → sink chain, shares one reconnecting client across players on a page, and reports `statechange`, `error` and `warning`.
- An element runtime (`defineElement`, source/transform/sink base classes, pads with backpressure) for custom elements.
- Built-in catalog source with delta updates, a CMAF source with seeking, URL ranges and rendition switching, and an MSE sink.
- Media and event timelines and per-request object ordering.
- Plugins (`definePlugin`, `opts.plugins`) that add setup parameters, check the peer's SETUP, and offer elements, switching set sources and buffer controllers. Built in: `ssts()` and `liveBuffer()`.
- `parseMsfUrl` and `formatMsfUrl` for `moqt://...#msf:` URLs.
