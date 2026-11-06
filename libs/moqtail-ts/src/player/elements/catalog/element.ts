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
 * The catalog element: keeps the catalog current from the catalog track, loads the default
 * sources for the first catalog, and hands later updates to them (`PLAN.md` › Source
 * Elements, Proper Catalog Parsing).
 */

import { Location } from '@/model/common/location'
import type { Tuple } from '@/model/common/tuple'
import { SourceElement, type BaseElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import { hasInterface, type SourceElementDefinition } from '@/player/element/define'
import {
  MEDIA_SOURCE,
  type CatalogReader,
  type CatalogTrack,
  type MediaSourceOptions,
  type TimelineReader,
} from '@/player/element/interfaces'
import { PlayerError } from '@/player/error'
import { trackIdentifier, type MsfSource, type TimeRange } from '@/player/msf_url'
import { backoffDelay, sleep, within } from '@/player/util/backoff'
import { CATALOG_PRIORITY, DEFAULT_CATALOG_TIMEOUT_MS, DEFAULT_MAX_CATALOG_BYTES } from '@/player/constants'
import { trackKey } from '@/player/util/track'
import { TrackReader } from '@/player/util/track_reader'
import { MediaTimeline } from '@/player/elements/mediatimeline'
import { TemplateTimeline } from '@/player/elements/mediatimeline/template'
import { applyDelta } from './delta'
import { diffTracks } from './diff'
import { parseCatalog, type ParseContext, type ParseResult } from './parse'
import { selectEventTimelines, selectMedia, withRole, type SelectedSet, type SelectionOptions } from './selection'
import type { Catalog as CatalogData } from './types'

export interface CatalogOptions {
  /** Passed to the media sources: switch with `SWITCH_FROM`. */
  switchFrom?: boolean
  /** The parsed catalog URL, from the player. */
  source?: MsfSource
}

/** A source the catalog loaded, and the set it holds now. */
interface LoadedSet {
  element: BaseElement
  tracks: CatalogTrack[]
  /** The track the set was loaded with, whose grouping fields an added track must share. */
  sample: CatalogTrack
  /** What the probe picked; a switching set source may wrap it. */
  definition: SourceElementDefinition
  media: boolean
}

/** @public The catalog element, emitted with the player's `catalog` event. */
export class Catalog extends SourceElement implements CatalogReader {
  readonly url: string
  readonly namespace: Tuple
  readonly #options: CatalogOptions
  readonly #parse: ParseContext
  readonly #reader: TrackReader
  #catalog: CatalogData | undefined
  #stale = false
  #sets: LoadedSet[] = []
  /** Media timeline tracks that couldn't be read, by track key. */
  readonly #unavailableTimelines = new Set<string>()
  /** The first catalog with tracks or `isComplete`, or the fatal error that came first. */
  readonly #first = Promise.withResolvers<void>()
  #started = false
  /** Updates are reconciled one after another, since loading a source is async. */
  #updates = Promise.resolve()

  constructor(context: ElementContext, options: CatalogOptions & { source: MsfSource }) {
    super(context)
    const { source } = options
    this.url = source.url
    this.namespace = source.namespace
    this.#options = options
    this.#parse = { namespace: source.namespace, variables: source.variables }
    this.#first.promise.catch(() => {}) // read by start(), possibly after it settled
    this.#reader = new TrackReader(context, {
      track: { namespace: source.namespace, name: source.trackBytes },
      priority: CATALOG_PRIORITY,
      incremental: true,
      maxBytes: context.player.options.maxCatalogBytes ?? DEFAULT_MAX_CATALOG_BYTES,
      onIndependent: (payload) => this.#received(parseCatalog(payload, this.#parse)),
      onIncremental: (payload) => {
        if (!this.#catalog) throw new Error('A delta update arrived before any catalog')
        this.#received(applyDelta(this.#catalog, payload, this.#parse))
      },
      onStale: (reason) => this.#onStale(reason),
    })
  }

  /** The last good catalog's tracks, roles inferred. */
  get tracks(): readonly CatalogTrack[] {
    return this.#catalog?.tracks ?? []
  }

  get publishTracks(): readonly CatalogTrack[] {
    return this.#catalog?.publishTracks ?? []
  }

  get isComplete(): boolean {
    return this.#catalog?.isComplete === true
  }

  /** An update was missed or rejected since the last good catalog. */
  get stale(): boolean {
    return this.#stale
  }

  /** Over the tracks of the loaded sources. */
  get streamType(): 'unknown' | 'live' | 'on-demand' {
    const selected = this.#selected()
    if (selected.length === 0) return 'unknown'
    return selected.some((t) => t.isLive) ? 'live' : 'on-demand'
  }

  /** Seconds: the longest `trackDuration` of the loaded sources' tracks, `Infinity` while live, `NaN` unknown. */
  get duration(): number {
    if (this.streamType === 'live') return Infinity
    const durations = this.#selected().flatMap((t) => t.trackDuration ?? [])
    return durations.length > 0 ? Math.max(...durations) / 1000 : NaN
  }

  /** A track by name bytes, in the catalog's namespace unless given (MSF 11.2.2). */
  find(name: string | Uint8Array, namespace: Tuple = this.namespace): CatalogTrack | undefined {
    const key = trackIdentifier(namespace, typeof name === 'string' ? new TextEncoder().encode(name) : name)
    return this.tracks.find((t) => trackKey(t) === key)
  }

  byRole(role: string): CatalogTrack[] {
    return this.tracks.filter((t) => t.role === role)
  }

  /** Tracks referencing other catalogs (MSF 5.6.16); loading one is up to the user. */
  references(): CatalogTrack[] {
    return this.tracks.filter((t) => t.packaging === 'catalog')
  }

  /**
   * The track's `template`, which needs no subscription; else the media timeline track whose
   * `depends` names the track or its render group (MSF 7.2), loaded once and shared; else
   * undefined. A media timeline that can't be read is a warning, and undefined from then on,
   * so seeks don't wait for it again.
   */
  async timeline(track: CatalogTrack): Promise<TimelineReader | undefined> {
    const window = track.accessibleWindowTime
    if (track.template) return new TemplateTimeline(track.template, window)
    const timeline = this.tracks.find(
      (t) =>
        t.packaging === 'mediatimeline' &&
        t.namespace.equals(track.namespace) &&
        t.depends?.some((d) => d === track.name || (track.renderGroup !== undefined && d === track.renderGroup)),
    )
    const key = timeline && trackKey(timeline)
    if (!key || this.#unavailableTimelines.has(key)) return undefined
    try {
      return (await this.context.load(MediaTimeline, { tracks: [timeline] })).reader(window)
    } catch (error) {
      // Concurrent callers share the failed load: warn once.
      if (this.#unavailableTimelines.has(key)) return undefined
      this.#unavailableTimelines.add(key)
      const reason = error instanceof Error ? error.message : String(error)
      this.#warn(new PlayerError('TRACK_SKIPPED', `Media timeline unavailable: ${reason}`, { element: 'catalog' }))
      return undefined
    }
  }

  /**
   * Subscribes, waits for the first catalog with tracks or `isComplete`, loads the default
   * sources, and emits `nomorepads`.
   * @throws PlayerError `CATALOG_TIMEOUT` without a usable catalog in time, `UNSUPPORTED_SOURCE`
   * for an invalid first catalog or one with nothing playable.
   */
  async start(): Promise<void> {
    const timeoutMs = this.context.player.options.catalogTimeoutMs ?? DEFAULT_CATALOG_TIMEOUT_MS
    const startedAt = performance.now()
    const left = () => timeoutMs - (performance.now() - startedAt)
    const timedOut = (cause?: unknown) =>
      new PlayerError('CATALOG_TIMEOUT', `No catalog within ${timeoutMs} ms`, { element: 'catalog', cause })
    try {
      for (let attempt = 0; ; attempt++) {
        try {
          await this.#reader.start()
          break
        } catch (cause) {
          const wait = backoffDelay(attempt)
          if (wait > left()) throw timedOut(cause)
          await sleep(wait, this.context.signal)
        }
      }
      await within(this.#first.promise, left(), this.context.signal, timedOut)
      const catalog = this.#catalog!
      this.#started = true
      if (catalog.tracks.length > 0) {
        const loading = this.#loadDefaults(catalog)
        this.#updates = loading.catch(() => {})
        await loading
      }
    } catch (error) {
      await this.#reader.stop()
      throw error
    }
    this.emit('nomorepads')
  }

  override async destroy(): Promise<void> {
    await this.#reader.stop()
    await super.destroy()
  }

  #received({ catalog, warnings }: ParseResult): void {
    for (const warning of warnings) this.#warn(warning)
    const previous = this.#catalog
    const next = { ...catalog, tracks: catalog.tracks.map(withRole) }
    this.#catalog = next
    this.#stale = false
    if (!this.#started) {
      if (next.tracks.length > 0 || next.isComplete) this.#first.resolve()
      return this.#announce()
    }
    this.#updates = this.#updates
      .then(() => this.#reconcile(previous!, next))
      .catch((error) => this.#warn(asUpdateError(error)))
      .then(() => this.#announce())
  }

  #announce(): void {
    this.context.player.emit('catalog', this)
  }

  #onStale(reason: Error): void {
    this.#stale = true
    // No good catalog yet: it's fatal.
    if (!this.#catalog) {
      const fatal = reason instanceof PlayerError && reason.code === 'UNSUPPORTED_SOURCE'
      return this.#first.reject(
        fatal
          ? reason
          : new PlayerError('UNSUPPORTED_SOURCE', `Unusable catalog: ${reason.message}`, { cause: reason }),
      )
    }
    this.#warn(asUpdateError(reason))
  }

  #warn(warning: PlayerError): void {
    this.context.player.emit('warning', warning)
  }

  #selection(): SelectionOptions {
    const { preferredLanguage, eventTimelines } = this.context.player.options
    const language = preferredLanguage ?? globalThis.navigator?.language
    return {
      probe: (track) => this.context.probe(track),
      ...(language && { language }),
      ...(eventTimelines && { eventTimelines }),
    }
  }

  async #loadDefaults(catalog: CatalogData): Promise<void> {
    const selection = this.#selection()
    const media = selectMedia(catalog.tracks, selection)
    if (media.length === 0) {
      throw new PlayerError('UNSUPPORTED_SOURCE', 'No track in the catalog is playable', { element: 'catalog' })
    }
    const { sets: timelines, skipped } = selectEventTimelines(catalog.tracks, selection)
    for (const track of skipped) {
      const message = `No element handles event timeline "${track.name}" (${track.eventType})`
      this.#warn(new PlayerError('TRACK_SKIPPED', message, { element: 'catalog' }))
    }
    const reference = media.find((s) => s.tracks[0]!.role === 'video') ?? media[0]!
    const byTime = await this.#rangesByTime(reference.tracks[0]!)
    await Promise.all([
      ...media.map(async (s) => {
        // Another set takes the ranges by media time when it can map them back to its own groups.
        const mapped = s !== reference && byTime && (await this.timeline(s.tracks[0]!)) ? byTime : undefined
        await this.#load(s, true, mapped)
      }),
      ...timelines.map((s) => this.#load(s, false)),
    ])
  }

  /**
   * The URL's location ranges as media time on `reference`'s timeline. Sets outside its
   * alternate group needn't share its Group IDs (MSF 4.2), so for them a location range
   * only means the media time it covers on the reference. Undefined without location
   * ranges or a timeline: then every set takes them as written.
   */
  async #rangesByTime(reference: CatalogTrack): Promise<TimeRange[] | undefined> {
    const ranges = this.#options.source?.locationRanges ?? []
    const timeline = ranges.length > 0 ? await this.timeline(reference) : undefined
    if (!timeline) return undefined
    const times: TimeRange[] = []
    for (const { start, end } of ranges) {
      const from = timeline.locationToTime(new Location(start.group, 0n))
      // The end group runs until the next one starts.
      const to = end && timeline.locationToTime(new Location(end.group + 1n, 0n))
      if (from === undefined || (end && to === undefined)) return undefined
      times.push({ start: from, ...(to !== undefined && { end: to - 1 }) })
    }
    return times
  }

  /**
   * Loads a set: through the switching set source of a listed and negotiated extension, if any.
   * `byTime` replaces the URL's location ranges for a set that takes them as media time.
   */
  async #load(set: SelectedSet, media: boolean, byTime?: TimeRange[]): Promise<void> {
    const { tracks, definition } = set
    const parent = media
      ? this.context.player.options.extensions?.find(
          (e) => e.switchingSetSource && (e.negotiated?.(this.context.client.serverSetup) ?? true),
        )?.switchingSetSource
      : undefined
    const options = media ? this.#mediaOptions(byTime) : {}
    const element = parent
      ? await this.context.load(parent, { tracks, options: { ...options, child: definition }, catalog: this })
      : await this.context.load(definition, { tracks, options, catalog: this })
    this.#sets.push({ element, tracks, sample: tracks[0]!, definition, media })
  }

  #mediaOptions(byTime?: TimeRange[]): MediaSourceOptions {
    const { switchFrom, source } = this.#options
    const { wallclockRanges = [] } = source ?? {}
    const locationRanges = byTime ? [] : (source?.locationRanges ?? [])
    const mediaTimeRanges = [...(source?.mediaTimeRanges ?? []), ...(byTime ?? [])]
    const ranged = locationRanges.length + mediaTimeRanges.length + wallclockRanges.length > 0
    return {
      ...(switchFrom !== undefined && { switchFrom }),
      ...(ranged && { ranges: { locationRanges, mediaTimeRanges, wallclockRanges } }),
    }
  }

  /**
   * Gives each loaded media source its new set. An `isComplete` catalog removes nothing,
   * even with `tracks: []` (MSF 11.4). Added tracks join the set they fit; otherwise they
   * only start a source for a role that has none.
   */
  async #reconcile(previous: CatalogData, next: CatalogData): Promise<void> {
    const diff = diffTracks(previous.tracks, next.tracks)
    for (const warning of diff.warnings) this.#warn(warning)
    next.tracks = [...diff.tracks.values()]
    const removed = next.isComplete ? new Set<string>() : diff.removed
    const unplaced = [...diff.added]
    for (const set of this.#sets) {
      const tracks = set.tracks.flatMap((t) => (removed.has(trackKey(t)) ? [] : [diff.tracks.get(trackKey(t)) ?? t]))
      for (const track of set.media ? [...unplaced] : []) {
        // Kept by an isComplete catalog, which removes nothing: it's already in the set.
        const present = tracks.some((t) => trackKey(t) === trackKey(track))
        if (!present && !this.#fits(set, track)) continue
        if (!present) tracks.push(track)
        unplaced.splice(unplaced.indexOf(track), 1)
      }
      const changed =
        tracks.length !== set.tracks.length ||
        tracks.some((t, i) => {
          const key = trackKey(t)
          // Removed and still in the new catalog: it changed beyond what applies in place.
          return (
            key !== trackKey(set.tracks[i]!) || diff.updated.has(key) || (diff.removed.has(key) && diff.tracks.has(key))
          )
        })
      set.tracks = tracks
      if (changed && hasInterface(set.element, MEDIA_SOURCE)) set.element.setTracks(tracks)
    }
    if (unplaced.length > 0) await this.#loadMissingRoles(next)
  }

  /** Same grouping fields as the set's tracks, and the same probe result. */
  #fits(set: LoadedSet, track: CatalogTrack): boolean {
    const { sample } = set
    return (
      track.altGroup !== undefined &&
      (['role', 'renderGroup', 'altGroup', 'lang', 'packaging'] as const).every((f) => track[f] === sample[f]) &&
      !track.depends?.length &&
      this.context.probe(track) === set.definition
    )
  }

  /** A new source for each default set whose role has no source with tracks, replacing an emptied one. */
  async #loadMissingRoles(catalog: CatalogData): Promise<void> {
    const roleOf = (set: LoadedSet) => set.sample.role
    const playing = new Set(this.#sets.filter((s) => s.media && s.tracks.length > 0).map(roleOf))
    for (const set of selectMedia(catalog.tracks, this.#selection())) {
      const role = set.tracks[0]!.role
      if (playing.has(role)) continue
      for (const empty of this.#sets.filter((s) => s.media && roleOf(s) === role)) {
        this.#sets.splice(this.#sets.indexOf(empty), 1)
        await this.context.release(empty.element)
      }
      await this.#load(set, true)
    }
  }

  #selected(): CatalogTrack[] {
    return this.#sets.flatMap((s) => s.tracks)
  }
}

function asUpdateError(error: unknown): PlayerError {
  if (error instanceof PlayerError && error.code === 'INVALID_CATALOG_UPDATE') return error
  const message = `Catalog update rejected: ${error instanceof Error ? error.message : String(error)}`
  return new PlayerError('INVALID_CATALOG_UPDATE', message, { element: 'catalog', cause: error })
}
