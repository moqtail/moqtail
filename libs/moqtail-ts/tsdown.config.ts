/**
 * Copyright 2026 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { globSync, readFileSync, writeFileSync } from 'node:fs'
import { gzipSync } from 'node:zlib'
import { defineConfig, Rolldown } from 'tsdown'

interface Import {
  name: string
  from: string
}

interface Size {
  min: number
  gzip: number
}

/**
 * What an app pays for each import, measured by bundling a one-line app against `dist` the way
 * a bundler would. The docs' size table reads it from `build-info.json`.
 */
interface BuildInfo {
  version: string
  /** What each entry point's main import loads up front. */
  entries: (Import & { initial: Size })[]
  /** Gzip bytes each import adds on top of `MOQtailPlayer`. */
  additions: (Import & { gzip: number })[]
  /** The chunks `MOQtailPlayer` fetches when a stream first needs them, in gzip bytes. */
  lazy: { name: string; gzip: number }[]
}

type Chunk = Rolldown.OutputChunk

/** The elements with a lazy built-in (`meta.ts`), each exported bundled from its own entry point. */
const LAZY_ELEMENTS = globSync('src/player/elements/*/meta.ts')
  .sort()
  .map((meta) => meta.split('/').at(-2)!)

const PLAYER: Import = { name: 'MOQtailPlayer', from: 'moqtail/player' }

const ENTRIES: Import[] = [
  { name: 'MOQtailClient', from: 'moqtail/client' },
  PLAYER,
  { name: 'defineMOQtailVideo', from: 'moqtail/player/component' },
]

/** Every plugin, from `moqtail/player`, and every bundled element, from its own entry point. */
const ADDITIONS: Import[] = [
  ...globSync('src/player/plugins/*/index.ts')
    .sort()
    .flatMap((file) => exportsOf(file, 'definePlugin').map((name) => ({ name, from: 'moqtail/player' }))),
  ...LAZY_ELEMENTS.flatMap((element) =>
    exportsOf(`src/player/elements/${element}/index.ts`, 'defineElement').map((name) => ({
      name,
      from: `moqtail/player/elements/${element}`,
    })),
  ),
]

export default defineConfig({
  entry: {
    index: 'src/index.ts',
    client: 'src/client/index.ts',
    model: 'src/model/index.ts',
    player: 'src/player/index.ts',
    'player/component': 'src/player/component/index.ts',
    'player/worker': 'src/player/worker/index.ts',
    'player/worker-default': 'src/player/worker/default.ts',
    util: 'src/util/index.ts',
    ...Object.fromEntries(
      LAZY_ELEMENTS.map((name) => [`player/elements/${name}`, `src/player/elements/${name}/index.ts`]),
    ),
  },
  format: 'esm',
  dts: true,
  outDir: 'dist',
  sourcemap: false,
  clean: true,
  treeshake: true,
  define: {
    'import.meta.vitest': 'undefined',
  },
  target: false,
  hooks: {
    'build:done': async () => {
      const { entries } = await writeBuildInfo(LAZY_ELEMENTS)
      const player = entries.find((e) => e.name === PLAYER.name)!.initial
      console.log(`MOQtailPlayer: ${kb(player.min)} min │ gzip: ${kb(player.gzip)} (build-info.json)`)
    },
  },
})

/**
 * Writes `build-info.json` and returns it. Throws if one of `lazyElements` is in the main chunk,
 * which a static import of its entry point would do without any build warning.
 */
async function writeBuildInfo(lazyElements: string[]): Promise<BuildInfo> {
  const { version } = JSON.parse(readFileSync('package.json', 'utf8')) as { version: string }

  const entries: BuildInfo['entries'] = []
  for (const entry of ENTRIES) entries.push({ ...entry, initial: size(initialOf(await bundle([entry]))) })

  const player = await bundle([PLAYER])
  const initial = new Set(initialOf(player))
  const base = size([...initial]).gzip

  const additions: BuildInfo['additions'] = []
  for (const addition of ADDITIONS) {
    const added = size(initialOf(await bundle([PLAYER, addition]))).gzip - base
    additions.push({ ...addition, gzip: added })
  }

  const lazy = player.filter((c) => !initial.has(c)).map((c) => ({ name: c.name, gzip: size([c]).gzip }))

  // A static import turns the element's chunk into a facade of a few bytes over the main chunk
  const bundled = lazyElements.filter((name) => !lazy.some((c) => c.name === name && c.gzip > 500))
  if (bundled.length > 0) throw new Error(`MOQtailPlayer bundles lazy elements statically: ${bundled.join(', ')}`)

  const info: BuildInfo = { version, entries, additions, lazy }
  writeFileSync('build-info.json', `${JSON.stringify(info, null, 2)}\n`)
  return info
}

/** The chunks of an app that imports and uses `imports`. */
async function bundle(imports: Import[]): Promise<Chunk[]> {
  const code = [
    ...imports.map(({ name, from }) => `import { ${name} } from '${from}'`),
    `globalThis.used = [${imports.map(({ name }) => name).join(', ')}]`,
  ].join('\n')

  const { output } = await Rolldown.build({
    input: 'app',
    write: false,
    output: { minify: true },
    logLevel: 'silent',
    plugins: [
      {
        name: 'app',
        // Bare imports resolve from the package root, as `moqtail` itself
        resolveId: (id) => (id === 'app' ? id : undefined),
        load: (id) => (id === 'app' ? code : undefined),
      },
    ],
  })
  return output.filter((o): o is Chunk => o.type === 'chunk')
}

/** The entry chunk and what it imports statically: what the app loads before running. */
function initialOf(chunks: Chunk[]): Chunk[] {
  const byName = new Map(chunks.map((c) => [c.fileName, c]))
  const initial = new Set<Chunk>()

  const visit = (chunk: Chunk) => {
    if (initial.has(chunk)) return
    initial.add(chunk)
    for (const name of chunk.imports) visit(byName.get(name)!)
  }

  visit(chunks.find((c) => c.isEntry)!)
  return [...initial]
}

/** Minified and gzipped bytes. */
function size(chunks: Chunk[]): Size {
  const code = chunks.map((c) => c.code).join('')
  return { min: Buffer.byteLength(code), gzip: gzipSync(code).length }
}

/** The names `file` exports as `export const <name> = <call>(`, the form every plugin and element uses. */
function exportsOf(file: string, call: 'definePlugin' | 'defineElement'): string[] {
  const pattern = new RegExp(`^export const (\\w+) = ${call}\\(`, 'gm')
  return [...readFileSync(file, 'utf8').matchAll(pattern)].map((match) => match[1]!)
}

function kb(bytes: number): string {
  return `${(bytes / 1000).toFixed(2)} kB`
}
