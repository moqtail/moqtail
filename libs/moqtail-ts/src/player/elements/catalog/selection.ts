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

import { DEFAULT_PRIORITY, type SourceElementDefinition } from '@/player/element/define'
import type { CatalogTrack } from '@/player/element/interfaces'

/** The tracks one source is loaded for, and the definition their probe picked. */
export interface SelectedSet {
  definition: SourceElementDefinition
  tracks: CatalogTrack[]
}

export interface SelectionOptions {
  probe(track: CatalogTrack): SourceElementDefinition | undefined
  /** How many transforms the chain would load to play the track, undefined when it can't. Without it, 0. */
  cost?(track: CatalogTrack): number | undefined
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
export function isCandidate(t: CatalogTrack): boolean {
  return (t.role === 'video' || t.role === 'audio') && t.packaging !== 'catalog' && !t.depends?.length
}

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
 * The default video and audio sets, in that order, with their start tracks first. Tracks that can't
 * be played leave their set, and an emptied set is dropped. Of the sets sharing an `altGroup` and
 * `lang`, only those of the lowest cost are kept, then those whose definition has the lowest priority.
 */
export function selectMedia(tracks: readonly CatalogTrack[], options: SelectionOptions): SelectedSet[] {
  const cost = options.cost ?? (() => 0)
  const playable = mediaSets(tracks, options.probe).flatMap((s) => {
    const kept = s.tracks.filter((t) => cost(t) !== undefined)
    return kept.length > 0 ? [{ ...s, tracks: kept }] : []
  })

  // A set's tracks share a definition and a packaging, so they cost the same
  const cheapest = lowest(playable, (s) => cost(s.tracks[0]!)!)

  const sets = lowest(cheapest, (s) => s.definition.priority ?? DEFAULT_PRIORITY)
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

/** The sets of lowest `measure` among those sharing an `altGroup` and `lang`; sets without an `altGroup` all stay. */
function lowest(sets: SelectedSet[], measure: (set: SelectedSet) => number): SelectedSet[] {
  const key = (s: SelectedSet) => (s.tracks[0]!.altGroup === undefined ? undefined : setKey(s.tracks[0]!, 0))
  const best = new Map<string, number>()

  for (const s of sets) {
    const k = key(s)
    if (k !== undefined) best.set(k, Math.min(best.get(k) ?? Infinity, measure(s)))
  }

  return sets.filter((s) => key(s) === undefined || measure(s) === best.get(key(s)!))
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { Tuple } = await import('@/model/common/tuple')
  const { collectingSink, testSource, testTransform } = await import('@/player/testing/elements')
  const { routeOf, sourceCapabilities } = await import('@/player/element/negotiation')
  const { probeCmaf } = await import('@/player/elements/cmaf')
  const { Locmaf, probeLocmaf } = await import('@/player/elements/locmaf')
  type ElementDefinition = import('@/player/element/define').ElementDefinition
  type TransformElementDefinition = import('@/player/element/define').TransformElementDefinition

  const cmaf = testSource({ name: 'cmaf', packaging: 'cmaf', probe: probeCmaf })
  const locmaf = testSource({ name: 'locmaf', packaging: 'cmaf', probe: probeLocmaf, priority: Locmaf.priority! })
  const loc = testSource({ name: 'loc', packaging: 'loc', probe: (t) => t.packaging === 'loc' })
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

  /** Costs as the chain works them out for declared `sinks`, loading from `loadable`. */
  const costFor =
    (sinks: ElementDefinition[], loadable: TransformElementDefinition[] = []) =>
    (track: CatalogTrack) =>
      routeOf(sourceCapabilities(probe(track)!, track.role)!, { transforms: [], sinks }, loadable)?.loaded

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

      const cameras = [
        track('main_cam', { packaging: 'cmaf', altGroup: 1, ...video }),
        track('alt_cam', { packaging: 'locmaf', locmafVersion: '0.3', altGroup: 2, ...video }),
      ]
      expect(names(selectMedia(cameras, { probe }))).toEqual(['cmaf: main_cam'])
    })

    test('only sets a declared sink can reach, the fewest transforms first, then the lowest priority', () => {
      const video = { codec: 'avc1.64001f', height: 720, altGroup: 1 }
      const tracks = [
        track('v_loc', video),
        track('v_cmaf', { ...video, packaging: 'cmaf' }),
        track('v_locmaf', { ...video, packaging: 'locmaf', locmafVersion: '0.3' }),
      ]

      const mse = costFor([collectingSink({ packaging: 'cmaf' })])
      const canvas = costFor([collectingSink({ packaging: 'raw' })], [testTransform({ takes: 'loc', makes: 'raw' })])
      const pick = (sets: CatalogTrack[], cost?: SelectionOptions['cost']) =>
        names(selectMedia(sets, { probe, ...(cost && { cost }) }))

      expect([pick(tracks, mse), pick(tracks, canvas), pick(tracks)]).toEqual([
        ['locmaf: v_locmaf'],
        ['loc: v_loc'],
        ['locmaf: v_locmaf'],
      ])

      // In sets of their own, the first one in catalog order is dropped when no sink reaches it
      const apart = tracks.map((t, i) => ({ ...t, altGroup: i + 1 }))
      expect([pick(apart, mse), pick(apart)]).toEqual([['cmaf: v_cmaf'], ['loc: v_loc']])
    })

    test('a rendition that can’t be played leaves its set, and an emptied set gives way to the next of its role', () => {
      const video = { packaging: 'cmaf', height: 720 }
      const tracks = [
        track('hevc', { ...video, codec: 'hvc1.1.6.L93.B0', altGroup: 1 }),
        track('v1', { ...video, codec: 'avc1.64001f', altGroup: 2 }),
        track('v2', { ...video, codec: 'hvc1.1.6.L93.B0', altGroup: 2 }),
      ]

      const cost = (t: CatalogTrack) => (t.codec?.startsWith('hvc1') ? undefined : 0)
      expect(names(selectMedia(tracks, { probe, cost }))).toEqual(['cmaf: v1'])
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
