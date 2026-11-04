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
 * Delta updates (MSF 5.1.6, 5.3). Operations apply in order to a copy of the catalog
 * document, which is then validated again as a whole, so a delta either applies fully or
 * not at all. Deltas carry no `initDataList`: tracks resolve `initRef` against the catalog's.
 */

import * as v from 'valibot'
import { PlayerError } from '@/player/error'
import { buildCatalog, readJson, type ParseContext, type ParseResult } from './parse'
import { DeltaSchema } from './schema'
import { splitNamespace, toBytes } from './strings'
import type { Catalog } from './types'
import { trackIdentifier } from '@/player/msf_url'

type Track = Record<string, unknown>
type Operation = v.InferOutput<typeof DeltaSchema>['deltaUpdate'][number]

const rejected = (reason: string) => new PlayerError('INVALID_CATALOG_UPDATE', `Delta update rejected: ${reason}`)

/**
 * Applies a delta update object to `base`, which is left as it is.
 * @throws PlayerError `INVALID_CATALOG_UPDATE` when the delta, or the catalog it leads to, is invalid.
 */
export function applyDelta(base: Catalog, bytes: Uint8Array, ctx: ParseContext): ParseResult {
  try {
    return applyOperations(base, bytes, ctx)
  } catch (error) {
    // A bad delta only makes the catalog stale; it never is a fatal source error.
    if (error instanceof PlayerError && error.code === 'UNSUPPORTED_SOURCE') throw rejected(error.message)
    throw error
  }
}

function applyOperations(base: Catalog, bytes: Uint8Array, ctx: ParseContext): ParseResult {
  const warnings: PlayerError[] = []
  const delta = v.safeParse(DeltaSchema, readJson(bytes, ctx, warnings))
  if (!delta.success) throw rejected(v.summarize(delta.issues))
  const document = structuredClone(base.json)
  document['generatedAt'] = delta.output.generatedAt
  for (const operation of delta.output.deltaUpdate) apply(document.tracks, operation, ctx)
  return { catalog: buildCatalog(document, ctx), warnings }
}

/** The key of a raw track's full name, its namespace inherited when absent (MSF 5.2.2, 11.2.2). */
function keyOf(track: Track, ctx: ParseContext): string {
  const { name, namespace } = track as { name: string; namespace?: string }
  return trackIdentifier(namespace === undefined ? ctx.namespace : splitNamespace(namespace), toBytes(name))
}

function apply(tracks: Track[], { op, tracks: changes }: Operation, ctx: ParseContext): void {
  const indexOf = (key: string) => tracks.findIndex((track) => keyOf(track, ctx) === key)
  const add = (track: Track) => {
    if (indexOf(keyOf(track, ctx)) >= 0) throw rejected(`track ${JSON.stringify(track['name'])} already exists`)
    tracks.push(track)
  }
  for (const change of changes) {
    if (op === 'add') {
      add(change)
      continue
    }
    if (op === 'remove') {
      tracks.splice(existing(indexOf(keyOf(change, ctx)), change['name']), 1)
      continue
    }
    const { parentName, parentNamespace, ...fields } = change as Track & {
      parentName: string
      parentNamespace?: string
    }
    const namespace = parentNamespace === undefined ? ctx.namespace : splitNamespace(parentNamespace)
    const at = existing(indexOf(trackIdentifier(namespace, toBytes(parentName))), parentName)
    if (op === 'clone') {
      add({ ...tracks[at], ...fields })
    } else {
      // An update never renames the track.
      const { name: _name, namespace: _namespace, ...updates } = fields
      tracks[at] = { ...tracks[at], ...updates }
    }
  }
}

function existing(index: number, name: unknown): number {
  if (index < 0) throw rejected(`track ${JSON.stringify(name)} isn't in the catalog`)
  return index
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { Tuple } = await import('@/model/common/tuple')
  const { parseCatalog } = await import('./parse')
  const ctx: ParseContext = { namespace: Tuple.fromUtf8Path('live'), variables: {} }
  const bytes = (json: unknown) => new TextEncoder().encode(JSON.stringify(json))
  const video = { name: 'video', packaging: 'cmaf', role: 'video', isLive: true, initRef: 'i', bitrate: 1 }
  const base = parseCatalog(
    bytes({
      version: 'draft-01',
      tracks: [video, { ...video, name: 'slides' }],
      initDataList: [{ id: 'i', type: 'inline', data: btoa('init') }],
    }),
    ctx,
  ).catalog
  const delta = (...deltaUpdate: unknown[]) => applyDelta(base, bytes({ generatedAt: 2, deltaUpdate }), ctx)
  const names = (catalog: Catalog) => catalog.tracks.map((t) => t.name)

  describe('applyDelta', () => {
    test('removes tracks (MSF 5.6.5)', () => {
      const { catalog } = delta({ op: 'remove', tracks: [{ name: 'video' }, { name: 'slides' }] })
      expect(catalog.tracks).toEqual([])
      expect(catalog.generatedAt).toBe(2)
      expect(base.tracks).toHaveLength(2)
    })

    test('adds, clones, and updates in order', () => {
      const { catalog } = delta(
        { op: 'update', tracks: [{ parentName: 'video', bitrate: 4, name: 'renamed' }] },
        { op: 'add', tracks: [{ ...video, name: 'extra', namespace: 'other', initRef: undefined }] },
        { op: 'clone', tracks: [{ parentName: 'video', name: 'video-720', width: 1280 }] },
      )
      expect(names(catalog)).toEqual(['video', 'slides', 'extra', 'video-720'])
      const clone = catalog.tracks[3]!
      expect([clone.bitrate, clone.width, clone.init]).toEqual([4, 1280, catalog.tracks[0]!.init])
      expect(clone['parentName']).toBeUndefined()
      expect(catalog.tracks[2]!.namespace.fields.map((f) => f.toUtf8())).toEqual(['other'])
    })

    test('rejects the whole delta on any failure', () => {
      const code = (run: () => unknown) => {
        try {
          run()
        } catch (error) {
          return (error as PlayerError).code
        }
        return 'no error'
      }
      const fails = (...ops: unknown[]) => expect(code(() => delta(...ops))).toBe('INVALID_CATALOG_UPDATE')
      fails({ op: 'add', tracks: [{ ...video }] })
      fails({ op: 'remove', tracks: [{ name: 'nope' }] })
      fails({ op: 'remove', tracks: [{ name: 'video', bitrate: 1 }] })
      fails({ op: 'clone', tracks: [{ parentName: 'video' }] })
      fails({ op: 'clone', tracks: [{ parentName: 'video', parentNamespace: 'other', name: 'x' }] })
      fails({ op: 'update', tracks: [{ name: 'video', bitrate: 4 }] })
      fails({ op: 'update', tracks: [{ parentName: 'video', depends: ['nope'] }] })
      fails({ op: 'update', tracks: [{ parentName: 'video', targetLatency: 1, buffers: {} }] })
      fails({ op: 'rename', tracks: [] })
      expect(() => applyDelta(base, bytes({ generatedAt: 2, deltaUpdate: [] }), ctx)).toThrow(/deltaUpdate/)
      expect(code(() => applyDelta(base, bytes({ version: 'draft-01', tracks: [] }), ctx))).toBe(
        'INVALID_CATALOG_UPDATE',
      )
      expect(code(() => applyDelta(base, bytes('{'), ctx))).toBe('INVALID_CATALOG_UPDATE')
      expect(names(base)).toEqual(['video', 'slides'])
    })
  })
}
