---
description: The gzipped size each moqtail import adds to a web app, measured from the current build of the package.
---

<script setup>
import { data } from './bundle-size.data'

const kB = (bytes) => `${(bytes / 1000).toFixed(1)} kB`
</script>

# Bundle size

What an app downloads for each import of `moqtail` {{ data.version }}. Each number comes from a one-line app that imports and uses the import, bundled with Rolldown, minified and gzipped. Your bundler and server compression change the numbers a little.

## Entry points

What the app loads before it runs, with the client and everything the import needs.

<table>
  <thead><tr><th>Import</th><th>From</th><th>Minified</th><th>Gzipped</th></tr></thead>
  <tbody>
    <tr v-for="e in data.entries" :key="e.name">
      <td><code>{{ e.name }}</code></td><td><code>{{ e.from }}</code></td>
      <td>{{ kB(e.initial.min) }}</td><td>{{ kB(e.initial.gzip) }}</td>
    </tr>
  </tbody>
</table>

`defineMOQtailVideo` includes the `liveBuffer()` and `abr()` plugins, which `<moqtail-video>` installs.

## Loaded on first use

Chunks `MOQtailPlayer` fetches when a stream first needs them: `mse` for a `<video>`, `playout` and `webcodecs` for a `<canvas>`, and `locmaf` for LOCMAF tracks. Your bundler emits them as separate files. Importing an element from its entry point bundles it with your app instead.

<table>
  <thead><tr><th>Element</th><th>Bundled from</th><th>Gzipped</th></tr></thead>
  <tbody>
    <tr v-for="c in data.lazy" :key="c.name">
      <td><code>{{ c.name }}</code></td><td><code>moqtail/player/elements/{{ c.name }}</code></td><td>{{ kB(c.gzip) }}</td>
    </tr>
  </tbody>
</table>

## Added to `MOQtailPlayer`

What each import adds to the app's initial download. An element imported from `moqtail/player/elements/*` is no longer fetched on first use.

<table>
  <thead><tr><th>Import</th><th>From</th><th>Adds, gzipped</th></tr></thead>
  <tbody>
    <tr v-for="a in data.additions" :key="a.name">
      <td><code>{{ a.name }}</code></td><td><code>{{ a.from }}</code></td><td>{{ kB(a.gzip) }}</td>
    </tr>
  </tbody>
</table>

## More information

- [Entry points](/ts/#entry-points)
- [Bundle the sinks and decoder](/ts/player/player#bundle-the-sinks-and-decoder)
