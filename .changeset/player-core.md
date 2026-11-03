---
'moqtail': minor
---

`moqtail/player` exports `MOQtailPlayer` and `defineExtension`. Players on a page share one client through a global slot, which reconnects with backoff (paused while the page is hidden or offline) and reports `UNSUPPORTED_ORIGIN` for a different relay or setup. The player builds its chain with `source()`, `transform()`, and `sink()`, follows the state table through `statechange`, reports `error` and `warning`, and forwards `play`, `pause`, `seek`, and `setPlaybackRate` to its media sink.
