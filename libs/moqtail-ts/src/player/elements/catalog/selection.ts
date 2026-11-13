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
 * Default source selection: one video and one audio switching
 * set from the first render group, plus the event timelines the player opted into.
 */

import type { SourceElementDefinition } from '@/player/element/define'
import type { CatalogTrack } from '@/player/element/interfaces'

/** The tracks one source is loaded for, and the definition their probe picked. */
export interface SelectedSet {
  definition: SourceElementDefinition
  tracks: CatalogTrack[]
}

export interface SelectionOptions {
  probe(track: CatalogTrack): SourceElementDefinition | undefined
  /** BCP 47 tag, e.g. `opts.preferredLanguage ?? navigator.language`. */
  language?: string
  eventTimelines?: string[] | 'all'
}

/** A track with its `role` inferred when the catalog leaves it out: MIME type, then video or audio fields. */
export function withRole(track: CatalogTrack): CatalogTrack {
  if (track.role !== undefined) return track
  const prefix = track.mimeType?.split('/')[0]
  const role =
    prefix === 'video' || prefix === 'audio'
      ? prefix
      : track.width !== undefined || track.height !== undefined
        ? 'video'
        : track.samplerate !== undefined || track.channelConfig !== undefined
          ? 'audio'
          : undefined
  return role ? { ...track, role } : track
}

/** Selectable media: video or audio, not a catalog reference, independently decodable. */
const isCandidate = (t: CatalogTrack) =>
  (t.role === 'video' || t.role === 'audio') && t.packaging !== 'catalog' && !t.depends?.length

/** Tracks with the same `altGroup` and `lang` switch together; no `altGroup` is a set of its own. */
const setKey = (t: CatalogTrack, index: number) => (t.altGroup === undefined ? `#${index}` : `${t.altGroup}|${t.lang}`)

/**
 * The media sets of the first render group, in catalog order, each probed and split so all
 * its tracks share a definition and a packaging. Tracks without a definition are left out.
 * Roles must already be inferred ({@link withRole}).
 */
export function mediaSets(tracks: readonly CatalogTrack[], probe: SelectionOptions['probe']): SelectedSet[] {
  const candidates = tracks.filter(isCandidate)
  if (candidates.length === 0) return []
  const renderGroup = candidates[0]!.renderGroup
  const groups = new Map<string, CatalogTrack[]>()
  candidates
    .filter((t) => t.renderGroup === renderGroup)
    .forEach((t, i) => {
      const key = setKey(t, i)
      groups.set(key, [...(groups.get(key) ?? []), t])
    })
  const sets: SelectedSet[] = []
  for (const group of groups.values()) {
    const split: SelectedSet[] = []
    for (const track of group) {
      const definition = probe(track)
      if (!definition) continue
      const set = split.find((s) => s.definition === definition && s.tracks[0]!.packaging === track.packaging)
      if (set) set.tracks.push(track)
      else split.push({ definition, tracks: [track] })
    }
    sets.push(...split)
  }
  return sets
}

/** The audio set for `language`: exact tag, then primary subtag, then the first. */
export function pickAudio(sets: SelectedSet[], language: string | undefined): SelectedSet | undefined {
  const lang = (s: SelectedSet) => s.tracks[0]!.lang?.toLowerCase()
  const wanted = language?.toLowerCase()
  const primary = wanted?.split('-')[0]
  return (
    sets.find((s) => wanted && lang(s) === wanted) ??
    sets.find((s) => primary && lang(s)?.split('-')[0] === primary) ??
    sets[0]
  )
}

/**
 * Puts the track with the lowest `keyframeSpacing` first, since sources start on the first
 * track and it switches soonest (MSF 6.3.1). Without any, catalog order stays.
 */
export function startFirst(tracks: CatalogTrack[]): CatalogTrack[] {
  const spaced = tracks.filter((t) => t.keyframeSpacing !== undefined)
  if (spaced.length === 0) return tracks
  const start = spaced.reduce((a, b) => (b.keyframeSpacing! < a.keyframeSpacing! ? b : a))
  return [start, ...tracks.filter((t) => t !== start)]
}

/**
 * The default video and audio sets, in that order, with their start tracks first. LOCMAF sets
 * come before the others: where a catalog offers both, it's the same media for less overhead.
 */
export function selectMedia(tracks: readonly CatalogTrack[], options: SelectionOptions): SelectedSet[] {
  const isLocmaf = (s: SelectedSet) => (s.tracks[0]!.packaging === 'locmaf' ? 0 : 1)
  const sets = mediaSets(tracks, options.probe).sort((a, b) => isLocmaf(a) - isLocmaf(b))
  const video = sets.find((s) => s.tracks[0]!.role === 'video')
  const audio = pickAudio(
    sets.filter((s) => s.tracks[0]!.role === 'audio'),
    options.language,
  )
  return [video, audio].filter((s) => s !== undefined).map((s) => ({ ...s, tracks: startFirst(s.tracks) }))
}

/** The opted-in event timeline tracks, each on its own; `skipped` have no element. */
export function selectEventTimelines(
  tracks: readonly CatalogTrack[],
  options: SelectionOptions,
): { sets: SelectedSet[]; skipped: CatalogTrack[] } {
  const wanted = options.eventTimelines ?? []
  const matched = tracks.filter(
    (t) => t.packaging === 'eventtimeline' && (wanted === 'all' || wanted.includes(t.eventType ?? '')),
  )
  const sets: SelectedSet[] = []
  const skipped: CatalogTrack[] = []
  for (const track of matched) {
    const definition = options.probe(track)
    if (definition) sets.push({ definition, tracks: [track] })
    else skipped.push(track)
  }
  return { sets, skipped }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { Tuple } = await import('@/model/common/tuple')
  const { testSource } = await import('@/player/testing/elements')
  const { probeCmaf } = await import('@/player/elements/cmaf')
  const { probeLocmaf } = await import('@/player/elements/locmaf')

  const cmaf = testSource({ name: 'cmaf', probe: probeCmaf })
  const locmaf = testSource({ name: 'locmaf', probe: probeLocmaf })
  const loc = testSource({ name: 'loc', probe: (t) => t.packaging === 'loc' })
  const timeline = testSource({ name: 'sap', probe: (t) => t.eventType === 'org.ietf.moq.cmsf.sap' })
  const probe = (track: CatalogTrack) => [cmaf, locmaf, loc, timeline].find((d) => d.probe(track))
  const names = (sets: SelectedSet[]) => sets.map((s) => `${s.definition.name}: ${s.tracks.map((t) => t.name)}`)
  const track = (name: string, extra: Partial<CatalogTrack> = {}) =>
    withRole({
      name,
      nameBytes: new TextEncoder().encode(name),
      namespace: Tuple.fromUtf8Path('ns'),
      packaging: 'loc',
      isLive: true,
      ...extra,
    } as CatalogTrack)

  describe('selection', () => {
    test('withRole infers from the MIME type, then video and audio fields', () => {
      const base = { name: 'x', nameBytes: new Uint8Array(), namespace: Tuple.fromUtf8Path('ns'), packaging: 'cmaf' }
      const role = (extra: Partial<CatalogTrack>) => withRole({ ...base, isLive: true, ...extra } as CatalogTrack).role
      expect([
        role({ mimeType: 'audio/mp4', width: 1 }),
        role({ height: 1 }),
        role({ channelConfig: '2' }),
        role({ role: 'caption', width: 1 }),
        role({}),
      ]).toEqual(['audio', 'video', 'audio', 'caption', undefined])
    })

    test('a set is split by packaging, and the lowest keyframeSpacing starts', () => {
      const video = track('v', { packaging: 'cmaf', codec: 'avc1.64001f', width: 1280, height: 720 })
      const set = [
        { ...video, name: 'a', altGroup: 1, keyframeSpacing: 4 },
        { ...video, name: 'b', altGroup: 1, packaging: 'loc' },
        { ...video, name: 'c', altGroup: 1, keyframeSpacing: 1 },
      ]
      expect(names(selectMedia(set, { probe }))).toEqual(['cmaf: c,a'])
      expect(names(mediaSets(set, probe))).toEqual(['cmaf: a,c', 'loc: b'])
    })

    test('LOCMAF is preferred over CMAF, each in a set of its own', () => {
      const pair = (name: string, extra: Partial<CatalogTrack>) => [
        track(name, { packaging: 'cmaf', altGroup: 1, ...extra }),
        track(`${name}_locmaf`, { packaging: 'locmaf', locmafVersion: '0.3', altGroup: 1, ...extra }),
      ]
      const video = { codec: 'avc1.64001f', height: 720 }
      const audio = { codec: 'mp4a.40.2', channelConfig: '2', altGroup: 2 }
      const tracks = [...pair('v1', video), ...pair('v2', video), ...pair('a', audio)]
      expect(names(mediaSets(tracks, probe))).toEqual([
        'cmaf: v1,v2',
        'locmaf: v1_locmaf,v2_locmaf',
        'cmaf: a',
        'locmaf: a_locmaf',
      ])
      expect(names(selectMedia(tracks, { probe }))).toEqual(['locmaf: v1_locmaf,v2_locmaf', 'locmaf: a_locmaf'])
      // An unsupported version leaves CMAF
      const old = tracks.map((t) => (t.packaging === 'locmaf' ? { ...t, locmafVersion: '0.2' } : t))
      expect(names(selectMedia(old, { probe }))).toEqual(['cmaf: v1,v2', 'cmaf: a'])
    })

    test('audio: exact tag, then primary subtag, then the first set', () => {
      const audio = (lang: string) => ({ definition: cmaf, tracks: [{ lang } as CatalogTrack] })
      const sets = [audio('en'), audio('fr-FR'), audio('fr-CA')]
      expect([
        pickAudio(sets, 'fr-CA'),
        pickAudio(sets, 'FR-be'),
        pickAudio(sets, 'de'),
        pickAudio(sets, undefined),
      ]).toEqual([sets[2], sets[1], sets[0], sets[0]])
    })

    test('event timelines are opt-in by eventType; unmatched ones are skipped', () => {
      const event = { packaging: 'eventtimeline' }
      const tracks = [
        track('video', { height: 720 }),
        track('audio', { channelConfig: '2' }),
        track('sap', { ...event, eventType: 'org.ietf.moq.cmsf.sap' }),
        track('ads', { ...event, eventType: 'ads' }),
      ]
      expect(selectEventTimelines(tracks, { probe }).sets).toEqual([])
      const all = selectEventTimelines(tracks, { probe, eventTimelines: 'all' })
      expect([names(all.sets), all.skipped.map((t) => t.name)]).toEqual([['sap: sap'], ['ads']])
      const listed = selectEventTimelines(tracks, { probe, eventTimelines: ['ads'] })
      expect([listed.sets, listed.skipped.map((t) => t.name)]).toEqual([[], ['ads']])
    })
  })
}
