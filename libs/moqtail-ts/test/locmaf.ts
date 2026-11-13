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

/**
 * Loads the LOCMAF golden vectors in `dev/conformance/locmaf/`. Outside `src/` for the reason
 * `conformance.ts` gives: test blocks import it dynamically, so `fs` stays out of the bundle.
 */

import { readdirSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const VECTOR_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '../../../dev/conformance/locmaf')

/** One object of a case: where it sits, its payload, and the canonical chunk it decodes to. */
export interface VectorObject {
  group: bigint
  object: bigint
  payload: Uint8Array
  canonical: Uint8Array
}

export interface VectorCase {
  name: string
  init: Uint8Array
  /** In group and object order. */
  objects: VectorObject[]
}

export function locmafVectors(): VectorCase[] {
  return readdirSync(VECTOR_DIR, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map(({ name }) => {
      const dir = resolve(VECTOR_DIR, name)
      const objects = readdirSync(resolve(dir, 'objects'))
        .sort()
        .map((file) => {
          const [, group, object] = /^g(\d+)_o(\d+)\.locmafobj$/.exec(file)!
          return {
            group: BigInt(group!),
            object: BigInt(object!),
            payload: read(resolve(dir, 'objects', file)),
            canonical: read(resolve(dir, 'canonical', file.replace('.locmafobj', '.cmfc'))),
          }
        })
      return { name, init: read(resolve(dir, 'init.mp4')), objects }
    })
}

/** A plain `Uint8Array`, which compares equal to the decoder's output where a `Buffer` wouldn't. */
function read(path: string): Uint8Array {
  return new Uint8Array(readFileSync(path))
}
