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

import { gzipSync } from 'node:zlib'
import { defineConfig, Rolldown } from 'tsdown'

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
      const { output } = await Rolldown.build({
        input: 'dist/player.mjs',
        write: false,
        output: { minify: true },
      })
      const code = output[0].code
      const kb = (bytes: number) => `${(bytes / 1000).toFixed(2)} kB`
      console.log(`moqtail/player bundle: ${kb(Buffer.byteLength(code))} min │ gzip: ${kb(gzipSync(code).length)}`)
    },
  },
})
