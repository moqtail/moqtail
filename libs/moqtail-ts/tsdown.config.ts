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

import { defineConfig } from 'tsdown'

const shared = {
  dts: true,
  outDir: 'dist',
  sourcemap: false,
  treeshake: true,
  define: {
    'import.meta.vitest': 'undefined',
  },
  target: false,
} as const

export default defineConfig([
  {
    ...shared,
    entry: {
      index: 'src/index.ts',
      client: 'src/client/index.ts',
      model: 'src/model/index.ts',
      util: 'src/util/index.ts',
    },
    format: ['cjs', 'esm'],
    clean: true,
  },
  // `moqtail/mmtp` is ESM-only: its optional peer `@blockcast/mmt-container`
  // ships ESM only, so a CJS build of this entry could never load it.
  {
    ...shared,
    entry: { mmtp: 'src/mmtp/index.ts' },
    format: ['esm'],
    clean: false,
  },
])
