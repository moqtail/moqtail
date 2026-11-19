---
'moqtail': minor
---

The chain loads the transforms a pad needs to reach a sink, after the declared elements and in the fewest steps, so LOC catalogs play with `player.sink(canvas)`. The catalog keeps only the switching sets a declared sink can reach, and an element definition's optional `supports(track)` drops renditions it can't handle: `mse` and `webcodecs` ask the browser. Element contexts gain `cost(track)`.

Breaking: `player.transform()` takes a transform element definition and its options, like `sink()`, instead of an instance. The package is pre-1.0, so there is no compatibility overload.
