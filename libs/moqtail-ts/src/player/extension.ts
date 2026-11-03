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

/** Extensions bundle elements and setup options, so a protocol extension is one entry in `opts.extensions`. */

import { SetupOptions } from '../model/parameter/setup_option'
import { canonicalJson } from './client_slot'
import type { ElementDefinition } from './element/define'
import { PlayerError } from './error'

/** @public */
export interface ExtensionDefinition {
  readonly name: string
  /** Registered on every player that lists the extension, like `registerElement`. */
  readonly elements?: readonly ElementDefinition[]
  /** Merged into `client.setupOptions` before the client is created. */
  readonly setupOptions?: SetupOptions
}

/** @public Checks an extension definition and returns it. */
export function defineExtension(definition: ExtensionDefinition): ExtensionDefinition {
  if (typeof definition.name !== 'string' || !/^\S+$/.test(definition.name)) {
    throw new TypeError(`Extension "${String(definition.name)}": name must be non-empty and without whitespace`)
  }
  return definition
}

/**
 * `client.setupOptions` followed by each extension's. An extension setting a parameter
 * type to another value than an earlier entry throws `UNSUPPORTED_ORIGIN`.
 */
export function mergeSetupOptions(
  base: SetupOptions | undefined,
  extensions: readonly ExtensionDefinition[],
): SetupOptions | undefined {
  if (!extensions.some((e) => e.setupOptions)) return base
  const merged = new SetupOptions()
  const owners = new Map<bigint, { owner: string; value: string }>()
  for (const pair of base?.build() ?? []) {
    merged.addRaw(pair)
    if (!owners.has(pair.typeValue))
      owners.set(pair.typeValue, { owner: 'client.setupOptions', value: canonicalJson(pair.value) })
  }
  for (const extension of extensions) {
    for (const pair of extension.setupOptions?.build() ?? []) {
      const value = canonicalJson(pair.value)
      const existing = owners.get(pair.typeValue)
      if (existing && existing.value !== value) {
        throw new PlayerError(
          'UNSUPPORTED_ORIGIN',
          `${existing.owner} and extension "${extension.name}" set setup parameter 0x${pair.typeValue.toString(16)} to different values`,
        )
      }
      if (existing) continue
      owners.set(pair.typeValue, { owner: `extension "${extension.name}"`, value })
      merged.addRaw(pair)
    }
  }
  return merged
}
