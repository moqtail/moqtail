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
 * Comparing two catalogs' tracks. Tracks match by namespace and name bytes (MSF 11.2.2).
 * A track keeps its identity when only fields that don't change decoding or set membership
 * changed; any other change is a removal plus an addition.
 */

import { deepEqual } from '@/player/element/context'
import type { CatalogTrack } from '@/player/element/interfaces'
import { PlayerError } from '@/player/error'
import { trackKey } from '@/player/elements/cmaf/track'
import { TRACK_FIELD_NAMES } from './schema'

/** Fields a source applies in place. `isLive` only from true to false (MSF 11.4). */
const IN_PLACE = new Set([
  'bitrate',
  'avgBitrate',
  'label',
  'width',
  'height',
  'displayWidth',
  'displayHeight',
  'framerate',
  'isLive',
  'trackDuration',
  'targetLatency',
  'buffers',
  'accessibleWindowTime',
  'maxGrpSapStartingType',
  'maxObjSapStartingType',
  'generatedAt',
])
/** The identity, compared through the key, and the template, which never changes (MSF 7.4.2). */
const IGNORED = new Set(['name', 'nameBytes', 'namespace', 'template'])
const KNOWN = new Set([...TRACK_FIELD_NAMES, 'init'])

export interface TrackDiff {
  /** Keys of the tracks that left, or changed beyond what applies in place. */
  removed: Set<string>
  /** New tracks, and new versions of tracks in `removed`. */
  added: CatalogTrack[]
  /** Keys of the tracks updated in place. */
  updated: Set<string>
  /** Every track of the new catalog by key, a changed template put back. */
  tracks: Map<string, CatalogTrack>
  warnings: PlayerError[]
}

export function diffTracks(before: readonly CatalogTrack[], after: readonly CatalogTrack[]): TrackDiff {
  const previous = new Map(before.map((t) => [trackKey(t), t]))
  const diff: TrackDiff = { removed: new Set(), added: [], updated: new Set(), tracks: new Map(), warnings: [] }
  for (let track of after) {
    const key = trackKey(track)
    const old = previous.get(key)
    previous.delete(key)
    if (old && !deepEqual(old.template, track.template)) {
      const message = `The template of track "${track.name}" changed; it's immutable (MSF 7.4.2), so the change is ignored`
      diff.warnings.push(new PlayerError('INVALID_CATALOG_UPDATE', message, { element: 'catalog' }))
      const { template: _template, ...rest } = track
      track = old.template ? { ...rest, template: old.template } : (rest as CatalogTrack)
    }
    diff.tracks.set(key, track)
    if (!old) {
      diff.added.push(track)
      continue
    }
    const changed = [...new Set([...Object.keys(old), ...Object.keys(track)])].filter(
      (field) => !IGNORED.has(field) && !deepEqual(old[field], track[field]),
    )
    const inPlace = (field: string) => (IN_PLACE.has(field) && (field !== 'isLive' || old.isLive)) || !KNOWN.has(field) // extra fields apply in place
    if (changed.every(inPlace)) {
      if (changed.length > 0) diff.updated.add(key)
      continue
    }
    diff.removed.add(key)
    diff.added.push(track)
  }
  for (const key of previous.keys()) diff.removed.add(key)
  return diff
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafTrack } = await import('@/player/testing/cmaf')
  const summary = (diff: TrackDiff) => ({
    removed: [...diff.removed].map((k) => k.split('--')[1]),
    added: diff.added.map((t) => t.name),
    updated: [...diff.updated].map((k) => k.split('--')[1]),
  })
  const template = { startMediaTime: 0, deltaMediaTime: 2000, startLocation: [1, 0] } as never

  test('diffTracks: in place, remove and add, and an ignored template change', () => {
    const before = [
      cmafTrack('same'),
      cmafTrack('rate', { bitrate: 1 }),
      cmafTrack('vod', { isLive: true }),
      cmafTrack('live', { isLive: false }),
      cmafTrack('codec'),
      cmafTrack('extra'),
      cmafTrack('gone'),
      cmafTrack('tpl', { template }),
    ]
    const after = [
      cmafTrack('same'),
      cmafTrack('rate', { bitrate: 2, label: 'x' }),
      cmafTrack('vod', { isLive: false, trackDuration: 4000 }),
      cmafTrack('live', { isLive: true }),
      cmafTrack('codec', { codec: 'hvc1' }),
      cmafTrack('extra', { custom: 1 }),
      cmafTrack('tpl'),
      cmafTrack('new'),
    ]
    const diff = diffTracks(before, after)
    expect(summary(diff)).toEqual({
      removed: ['live', 'codec', 'gone'],
      added: ['live', 'codec', 'new'],
      updated: ['rate', 'vod', 'extra'],
    })
    expect(diff.warnings).toMatchObject([{ code: 'INVALID_CATALOG_UPDATE' }])
    expect([...diff.tracks.values()].find((t) => t.name === 'tpl')?.template).toBe(template)
  })
}
