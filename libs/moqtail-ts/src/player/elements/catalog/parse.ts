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
 * Catalog bytes to a {@link Catalog}: BOM check, strict UTF-8, JSON, variables, the schema,
 * then the checks that span tracks. Any failure rejects the whole catalog.
 */

import * as v from 'valibot'
import type { Tuple } from '@/model/common/tuple'
import { PlayerError } from '@/player/error'
import { trackIdentifier } from '@/player/msf_url'
import { trackKey } from '@/player/util/track'
import { catalogSchema } from './schema'
import { hasBom, invalidCatalog, toBytes } from './strings'
import type { Catalog, CatalogDocument, CatalogTrack } from './types'
import { substituteVariables } from './variables'

/** @public What a catalog is parsed against. */
export interface ParseContext {
  /** The catalog track's namespace, inherited by tracks without one. */
  namespace: Tuple
  /** From the catalog URL's fragment (MSF 5.4.2). */
  variables: Record<string, string>
}

/** @public A parsed catalog and the non-fatal problems found on the way. */
export interface ParseResult {
  catalog: Catalog
  warnings: PlayerError[]
}

/**
 * Parses an independent catalog object.
 * @throws PlayerError `UNSUPPORTED_SOURCE` when the catalog isn't valid.
 */
export function parseCatalog(bytes: Uint8Array, ctx: ParseContext): ParseResult {
  const warnings: PlayerError[] = []
  return { catalog: buildCatalog(readJson(bytes, ctx, warnings), ctx), warnings }
}

/** Bytes to JSON with variables substituted. */
export function readJson(bytes: Uint8Array, ctx: ParseContext, warnings: PlayerError[]): unknown {
  if (hasBom(bytes)) throw invalidCatalog('it starts with a byte order mark (MSF 11.2.1)')
  let json: unknown
  try {
    json = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes))
  } catch (cause) {
    throw invalidCatalog('not UTF-8 encoded JSON', cause)
  }
  return substituteVariables(json, ctx.variables, warnings)
}

/** Validates a substituted catalog document. Delta updates rebuild through this too. */
export function buildCatalog(json: unknown, ctx: ParseContext): Catalog {
  const result = v.safeParse(catalogSchema(ctx.namespace), json)
  if (!result.success) throw invalidCatalog(v.summarize(result.issues))
  const { output } = result
  for (const track of output.tracks) {
    const entry = output.initDataList?.find((e) => e.id === track.initRef)
    if (entry) track.init = entry.init
  }
  checkNames(output.tracks)
  return { ...output, json: json as CatalogDocument }
}

/** Unique full track names (MSF 5.2.3), and `depends` entries that resolve (MSF 5.2.15). */
function checkNames(tracks: CatalogTrack[]): void {
  const keys = new Set(tracks.map(trackKey))
  if (keys.size !== tracks.length) throw invalidCatalog('two tracks have the same full track name (MSF 5.2.3)')
  for (const track of tracks) {
    for (const dependency of track.depends ?? []) {
      const resolves =
        typeof dependency === 'string'
          ? keys.has(trackIdentifier(track.namespace, toBytes(dependency)))
          : tracks.some((other) => other.renderGroup === dependency && other.namespace.equals(track.namespace))
      if (!resolves) throw invalidCatalog(`dependency ${JSON.stringify(dependency)} doesn't resolve (MSF 5.2.15)`)
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { Tuple } = await import('@/model/common/tuple')
  const ctx: ParseContext = { namespace: Tuple.fromUtf8Path('live'), variables: {} }
  const video = { name: 'v', packaging: 'cmaf', role: 'video', isLive: true }
  const bytes = (json: unknown) => new TextEncoder().encode(JSON.stringify(json))
  const parse = (catalog: Record<string, unknown>) => parseCatalog(bytes({ version: 'draft-01', ...catalog }), ctx)

  describe('parseCatalog', () => {
    test('rejects a BOM, malformed JSON or UTF-8, and an invalid root', () => {
      const bom = new Uint8Array([0xef, 0xbb, 0xbf, ...bytes({ version: 'draft-01', tracks: [] })])
      expect(() => parseCatalog(bom, ctx)).toThrow(/byte order mark/)
      expect(() => parseCatalog(new Uint8Array([0x7b, 0xff, 0x7d]), ctx)).toThrow(/UTF-8 encoded JSON/)
      expect(() => parseCatalog(bytes('{'), ctx)).toThrow(PlayerError)
      expect(() => parse({ tracks: {} })).toThrow(/Invalid catalog/)
    })

    test('keeps unknown fields and the document', () => {
      const { catalog } = parse({ custom: 1, tracks: [{ ...video, x: 2 }] })
      expect(catalog.tracks[0]!['x']).toBe(2)
      expect(catalog['custom']).toBe(1)
      expect(catalog.json).toEqual({ version: 'draft-01', custom: 1, tracks: [{ ...video, x: 2 }] })
    })

    test('resolves initRef', () => {
      const { catalog } = parse({
        tracks: [
          { ...video, initRef: 'a' },
          { ...video, name: 'none' },
        ],
        initDataList: [{ id: 'a', type: 'object-property', data: '0F' }],
      })
      expect(catalog.tracks.map((t) => t.init)).toEqual([{ type: 'object-property', property: 0x0f }, undefined])
    })

    test('rejects the catalog for an invalid track, duplicates, and unresolved depends', () => {
      expect(() => parse({ tracks: [video, { ...video, name: 'w', isLive: undefined }] })).toThrow(/isLive/)
      expect(() => parse({ tracks: [{ ...video, name: 'v\uDEAD' }] })).toThrow(/surrogate/)
      expect(() => parse({ tracks: [video, { ...video, namespace: 'live' }] })).toThrow(/MSF 5.2.3/)
      expect(() => parse({ tracks: [{ ...video, depends: ['missing'] }] })).toThrow(/MSF 5.2.15/)
      expect(() => parse({ tracks: [{ ...video, depends: [3] }] })).toThrow(/MSF 5.2.15/)
      const other = { ...video, name: 't', namespace: 'other', depends: ['v'] }
      expect(() => parse({ tracks: [video, other] })).toThrow(/MSF 5.2.15/)
      const { catalog } = parse({
        tracks: [
          { ...video, renderGroup: 1 },
          { ...video, name: 't', depends: ['v', 1] },
        ],
      })
      expect(catalog.tracks).toHaveLength(2)
    })
  })
}
