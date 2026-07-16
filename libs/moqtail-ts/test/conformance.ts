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
 * Loads the shared conformance fixtures in `dev/conformance/draft18/`.
 *
 * The fixtures are normative for both this package and `moqtail-rs`; see the README
 * there. This module is the TypeScript half of "codepoints live in one place": the
 * values are never repeated here, only read.
 *
 * It lives outside `src/` on purpose. The suite is in-source (`includeSource`), so a
 * static import of this module from `src/` would put Node's `fs` and a path reaching
 * out of the package into the published bundle's module graph. Test blocks import it
 * dynamically instead, and `import.meta.vitest` is defined away at build time.
 */

import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/** The key this package is listed under in a fixture's `pending` markers. */
export const LANG = 'ts'

const FIXTURE_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '../../../dev/conformance/draft18')

/** One codepoint in a fixture: a name, a value, and whether a stack is exempt yet. */
export interface Entry {
  name: string
  /** Hex string, e.g. `"0x50"`. Use {@link codepoint} rather than parsing it again. */
  value: string
  reserved?: boolean
  pending?: Record<string, number>
  notes?: string
  stream?: string
  scope?: string
  spec?: string
}

/** A registry: the draft's table, plus what each stack still gets wrong. */
export interface Registry {
  source: string
  entries: Entry[]
  /** Codepoints a stack still defines that the draft does not. */
  not_in_draft?: Entry[]
  /** Deliberate, permanent divergence. Never asserted against the draft. */
  local_extensions?: Entry[]
}

export interface VarintVector {
  encoding: string
  value: string
  minimal: boolean
}
export interface VarintBoundary {
  value: string
  length: number
  encoding: string
}
export interface VarintNonMinimal {
  encoding: string
  value: string
}
export interface VarintPrefixShape {
  value: string
  mask: string
  prefix: string
}
export interface VarintTruncated {
  encoding: string
  reason: string
}
interface Section<T> {
  entries: T[]
}
export interface VarintFixture {
  vectors: Section<VarintVector>
  boundaries: Section<VarintBoundary>
  non_minimal: Section<VarintNonMinimal>
  prefix_shapes: Section<VarintPrefixShape>
  roundtrip_values: Section<string>
  truncated: Section<VarintTruncated>
}

function load<T>(name: string): T {
  return JSON.parse(readFileSync(resolve(FIXTURE_DIR, name), 'utf8')) as T
}

/**
 * `property_types.json`: the draft's own table, plus the provisional registrations other
 * moq drafts hold in the same number space.
 */
export interface PropertyTypes extends Registry {
  provisional: Registry
}

export const messageTypes = (): Registry => load<Registry>('message_types.json')
export const requestErrorCodes = (): Registry => load<Registry>('request_error_codes.json')
export const terminationCodes = (): Registry => load<Registry>('termination_codes.json')
export const streamResetCodes = (): Registry => load<Registry>('stream_reset_codes.json')
export const propertyTypes = (): PropertyTypes => load<PropertyTypes>('property_types.json')
export const varint = (): VarintFixture => load<VarintFixture>('varint.json')
export const parameterTypes = (): { setup_options: Registry; message_parameters: Registry } =>
  load<{ setup_options: Registry; message_parameters: Registry }>('parameter_types.json')

/** Parses a `"0x2F00"`-style fixture value. */
export function parseHex(value: string): bigint {
  if (!/^0x[0-9a-fA-F]+$/.test(value)) {
    throw new Error(`fixture value ${JSON.stringify(value)} must be a 0x-prefixed hex string`)
  }
  return BigInt(value)
}

/**
 * Parses a decimal fixture value. Varint values are strings because 2^64-1 does not
 * survive a JSON number — `JSON.parse` rounds it to 18446744073709552000.
 */
export function parseValue(value: string): bigint {
  return BigInt(value)
}

/** Decodes a lowercase-hex fixture byte sequence. */
export function parseBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) {
    throw new Error(`fixture encoding ${JSON.stringify(hex)} has an odd number of hex digits`)
  }
  const out = new Uint8Array(hex.length / 2)
  for (let i = 0; i < out.length; i++) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  }
  return out
}

/** The issue that will make this package conform, or undefined if it must conform now. */
export function pendingIssue(entry: Entry): number | undefined {
  return entry.pending?.[LANG]
}

/** `SUBSCRIBE_NAMESPACE` -> `SubscribeNamespace`. */
export function pascal(name: string): string {
  return name
    .split('_')
    .map((part) => (part ? part[0]!.toUpperCase() + part.slice(1).toLowerCase() : ''))
    .join('')
}

/**
 * Names an enum member from its spec name, PascalCasing it except where this package
 * spells it differently.
 */
export const pascalIdent =
  (exceptions: Record<string, string> = {}) =>
  (entry: Entry): string =>
    exceptions[entry.name] ?? pascal(entry.name)

/**
 * Names an enum member from its spec name verbatim, for the enums this package spells
 * in SCREAMING_SNAKE.
 */
export const specIdent = (entry: Entry): string => entry.name

/**
 * Asserts an enum against a fixture registry.
 *
 * `identOf` says what this package should call an entry; `nameOf` reports what it
 * actually parses a codepoint as, or undefined if it rejects it.
 */
export function assertRegistry(
  reg: Registry,
  identOf: (entry: Entry) => string,
  nameOf: (codepoint: bigint) => string | undefined,
): void {
  for (const entry of reg.entries) {
    const actual = nameOf(parseHex(entry.value))
    const expected = identOf(entry)
    const conforms = entry.reserved ? actual === undefined : actual === expected
    const issue = pendingIssue(entry)

    if (issue !== undefined) {
      if (conforms) {
        throw new Error(
          `${entry.name} (${entry.value}) is marked pending{ts:${issue}} but now conforms — ` +
            `delete the marker in the fixture (${reg.source})`,
        )
      }
    } else if (entry.reserved) {
      if (actual !== undefined) {
        throw new Error(
          `${entry.name} (${entry.value}) is RESERVED and must be rejected, but parsed as ` +
            `${actual} (${reg.source})`,
        )
      }
    } else if (actual !== expected) {
      throw new Error(
        `${entry.name} (${entry.value}) does not match the fixture: expected ${expected}, ` +
          `got ${actual} (${reg.source})`,
      )
    }
  }

  for (const entry of reg.not_in_draft ?? []) {
    const actual = nameOf(parseHex(entry.value))
    const issue = pendingIssue(entry)
    if (issue !== undefined) {
      if (actual === undefined) {
        throw new Error(
          `${entry.name} (${entry.value}) is listed in not_in_draft pending{ts:${issue}} but is ` +
            `already gone — delete the entry (${reg.source})`,
        )
      }
    } else if (actual !== undefined) {
      throw new Error(
        `${entry.name} (${entry.value}) is not in the draft and must be rejected, but parsed ` +
          `as ${actual} (${reg.source})`,
      )
    }
  }
}
