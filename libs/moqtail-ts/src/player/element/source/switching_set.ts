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
 * `SwitchingSetSource`: one switching set on one source pad, and its renditions, whatever its
 * packaging. Subclasses answer what depends on the packaging through `Packaging`.
 */

import { Location } from '@/model/common/location'
import type { MoqtObject } from '@/model/data/object'
import type { TrackProperty } from '@/model/property/track_property'
import { SourceElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import type { Capabilities } from '@/player/element/define'
import type { CatalogTrack, MediaSourceElement, MediaSourceOptions, Rendition } from '@/player/element/interfaces'
import type { Pad } from '@/player/element/pad'
import { PlayerError } from '@/player/error'
import { bitrateOf, rendition, renditionId, trackKey } from '@/player/util/track'
import { Delivery } from './delivery'
import { toSpans, type Span } from './ranges'
import { closestBitrate, Recovery } from './recovery'
import { ladderOf, Switcher } from './switch'

/** What a switching set source can't know without its packaging. */
export interface Packaging {
  /** The source pad's capabilities for a set of `role`. Called from the constructor, so it can't read fields. */
  padCapabilities(role: string): Capabilities
  /** The pad's `mimeType` for `track`. */
  mimeType(track: CatalogTrack): string
  /**
   * Init data known once a request is accepted, from the catalog or the track properties.
   * @throws when the track's init data is missing or invalid: the request counts as rejected.
   */
  acceptedInitData(track: CatalogTrack, trackProperties: TrackProperty[]): Uint8Array | undefined
  /** Init data `object` carries, if any. */
  objectInitData(track: CatalogTrack, object: MoqtObject): Uint8Array | undefined
  /** Whether the track's packets can't be decoded before the first `objectInitData`. */
  initOnObjects(track: CatalogTrack): boolean
  /** The media time of `object` in seconds, given the init data the pad carries; undefined if unknown. */
  mediaTime(track: CatalogTrack, object: MoqtObject, initData: Uint8Array | undefined): number | undefined
  /** The object the pad carries for `object`, given the init data that applies to it; undefined drops it. */
  packet(track: CatalogTrack, object: MoqtObject, initData: Uint8Array | undefined): MoqtObject | undefined
}

/** What `planSource` worked out before the source starts: the URL ranges and their duration. */
interface SourcePlan {
  spans: readonly Span[]
  /** The end of the last range in seconds, when known. */
  duration?: number
}

/**
 * Plays a switching set (`context.tracks`, at least one track) from its first track:
 * live with SUBSCRIBE from the next group, VOD with FETCH from the start, or the URL ranges.
 * In a fast switching set each GoP ladder is one rendition, named after its longest GoP:
 * `keyframeSpacing` stays internal (draft-ugur-moq-fast-switching).
 */
export abstract class SwitchingSetSource extends SourceElement implements MediaSourceElement, Packaging {
  readonly pad: Pad
  #tracks: readonly CatalogTrack[]
  /** Rendition ids ABR may choose from. */
  #selected: Set<string>
  readonly #delivery: Delivery
  readonly #recovery: Recovery
  readonly #switcher: Switcher
  /** The track key of the rendition last announced. */
  #shown: string

  constructor(
    context: ElementContext,
    readonly options: MediaSourceOptions,
    plan: SourcePlan = { spans: [] },
  ) {
    super(context)
    const start = context.tracks[0]!
    this.#tracks = context.tracks
    this.#selected = new Set(context.tracks.map((t) => this.#id(t)))
    this.pad = this.createPad('src', this.padCapabilities(start.role ?? 'video'))
    this.#delivery = new Delivery(this.pad, context, options.requests, this)
    this.#delivery.duration = plan.duration
    this.#recovery = new Recovery(
      context,
      this.#delivery,
      {
        tracks: () => this.#tracks,
        candidates: () => this.#tracks.filter((t) => this.#isSelected(t)),
        giveUp: () => this.removePad(this.pad, true),
      },
      start,
      plan,
    )
    this.#switcher = new Switcher({
      context,
      options,
      delivery: this.#delivery,
      recovery: this.#recovery,
      tracks: () => this.#tracks,
      element: () => this.definition.name,
    })
    this.#shown = trackKey(this.#head(start))
    this.#delivery.onTrackStart = (track, changed) => this.#trackStarted(track, changed)
    this.pad.onSignal((signal) => {
      if (signal.type !== 'seek') return false
      this.#recovery.seek(signal.mediaTime)
      return true
    })
    this.emit('nomorepads')
    void this.#recovery.run()
  }

  abstract padCapabilities(role: string): Capabilities
  abstract mimeType(track: CatalogTrack): string
  abstract acceptedInitData(track: CatalogTrack, trackProperties: TrackProperty[]): Uint8Array | undefined
  abstract objectInitData(track: CatalogTrack, object: MoqtObject): Uint8Array | undefined
  abstract initOnObjects(track: CatalogTrack): boolean
  abstract mediaTime(track: CatalogTrack, object: MoqtObject, initData: Uint8Array | undefined): number | undefined
  abstract packet(track: CatalogTrack, object: MoqtObject, initData: Uint8Array | undefined): MoqtObject | undefined

  getRenditions(): Rendition[] {
    return this.#tracks.filter((t) => this.#head(t) === t).map((track) => this.#rendition(track))
  }

  /** Switches at the next switch point; resolves once the new track's first packet is written. */
  async switch(renditionId: string): Promise<void> {
    const track = this.#tracks.find((t) => this.#id(t) === renditionId)
    if (!track)
      throw new PlayerError('UNSUPPORTED_SOURCE', `Unknown rendition ${renditionId}`, { element: this.definition.name })
    const active = this.#recovery.active
    // Within the active rendition's ladder the climb already moves to its longest GoP
    const sameLadder = this.#head(track) === this.#head(active) && this.#tracks.includes(active)
    if (track !== active && sameLadder) return
    return this.#switcher.switch(track)
  }

  /**
   * Sets which renditions ABR may choose from. A deselected active rendition switches to the
   * closest selected bitrate; an empty selection stops requesting until a later one.
   */
  async selectRenditions(renditionIds: string[]): Promise<void> {
    this.#selected = new Set(renditionIds)
    this.context.player.emit('selectionchange', this)
    const selected = this.#tracks.filter((t) => this.#isSelected(t))
    if (selected.length === 0) {
      this.#recovery.pause()
      return
    }
    this.#recovery.resume()
    const active = this.#recovery.active
    if (!this.#isSelected(active)) await this.switch(this.#id(closestBitrate(selected, bitrateOf(active))!))
  }

  /**
   * Replaces the switching set. New renditions start selected, unless exactly one is selected.
   * A removed active track switches to the closest selected bitrate; an empty set removes the pad.
   */
  setTracks(tracks: readonly CatalogTrack[]): void {
    const known = new Set(this.#tracks.map((t) => this.#id(t)))
    const keepSelection = this.#selected.size === 1
    if (!keepSelection) {
      for (const track of tracks) if (!known.has(this.#id(track))) this.#selected.add(this.#id(track))
    }
    this.#tracks = tracks
    if (tracks.length === 0) {
      this.#recovery.pause()
      if (this.pads.includes(this.pad)) this.removePad(this.pad)
      return
    }
    const key = trackKey(this.#recovery.active)
    // The same track may come back with changes made in place, such as isLive turning false
    const active = tracks.find((t) => trackKey(t) === key)
    if (active) {
      this.#recovery.active = active
      return
    }
    const selected = tracks.filter((t) => this.#isSelected(t))
    const next = closestBitrate(selected.length > 0 ? selected : tracks, bitrateOf(this.#recovery.active))!
    void this.switch(this.#id(next)).catch(() => {})
  }

  #trackStarted(track: CatalogTrack, changed: boolean): void {
    // Only another track replaces it: handovers and seeks compare the active track by identity
    if (trackKey(track) !== trackKey(this.#recovery.active)) this.#recovery.active = track
    this.#recovery.dropHandover(track)
    this.#switcher.started(track)
    const head = this.#head(track)
    const shown = trackKey(head)
    // A climb up the ladder is no change of rendition
    if (changed && shown !== this.#shown) this.context.player.emit('renditionchange', this, this.#rendition(head))
    this.#shown = shown
    this.#switcher.upgrade()
  }

  /** The track `track`'s rendition stands for: the longest GoP of its ladder, which switches climb to. */
  #head(track: CatalogTrack): CatalogTrack {
    return ladderOf(track, this.#tracks).reduce(
      (a, b) => ((b.keyframeSpacing ?? 0) > (a.keyframeSpacing ?? 0) ? b : a),
      track,
    )
  }

  #isSelected(track: CatalogTrack): boolean {
    return this.#selected.has(this.#id(this.#head(track)))
  }

  /** `track` is the head of its ladder. */
  #rendition(track: CatalogTrack): Rendition {
    return rendition(track, this.context.catalog, {
      active: trackKey(track) === trackKey(this.#head(this.#recovery.active)),
      selected: this.#selected.has(this.#id(track)),
    })
  }

  #id(track: CatalogTrack): string {
    return renditionId(track, this.context.catalog)
  }
}

/**
 * The URL ranges of a switching set and their duration, from its first track's timeline.
 * @throws PlayerError `UNSUPPORTED_SOURCE` without tracks, or for ranges `toSpans` rejects.
 */
export async function planSource(
  context: ElementContext,
  options: MediaSourceOptions | undefined,
  element: string,
): Promise<SourcePlan> {
  const first = context.tracks[0]
  if (!first) throw new PlayerError('UNSUPPORTED_SOURCE', 'The source needs at least one track', { element })
  const timeline = await context.catalog?.timeline(first)
  const spans = toSpans(options?.ranges, first, timeline)
  const end = spans.at(-1)?.end
  const endMs = end && timeline?.locationToTime(new Location(end.group + 1n, 0n))
  return { spans, ...(endMs !== undefined && { duration: endMs / 1000 }) }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafChain, cmafTrack, fakeCatalog } = await import('@/player/testing/cmaf')
  const { flush, track } = await import('@/player/testing/elements')

  test('renditions: id, src, bitrate, and active and selected state, kept on setTracks', async () => {
    const other = {
      ...cmafTrack('v2', { bitrate: 2000, avgBitrate: 1800, width: 1280, framerate: 30 }),
      namespace: track('x', 'live').namespace,
    }
    const c = await cmafChain([cmafTrack('v1', { bitrate: 800 }), other], { catalog: fakeCatalog() })
    await flush()
    expect(c.source.getRenditions()).toEqual([
      {
        id: 'v1',
        src: 'moqt://relay.test/app#msf:ns--v1',
        bitrate: 800,
        codec: 'avc1.64001f',
        active: true,
        selected: true,
      },
      {
        id: 'live--v2',
        src: 'moqt://relay.test/app#msf:live--v2',
        bitrate: 1800,
        codec: 'avc1.64001f',
        width: 1280,
        frameRate: 30,
        active: false,
        selected: true,
      },
    ])
    await c.source.selectRenditions(['v1'])
    c.source.setTracks([cmafTrack('v1', { bitrate: 900 }), cmafTrack('v3')])
    expect(c.source.getRenditions().map((r) => [r.id, r.bitrate, r.active, r.selected])).toEqual([
      ['v1', 900, true, true],
      // Exactly one rendition was selected: a new one doesn't join the selection
      ['v3', 0, false, false],
    ])
  })

  test('a fast switching set has one rendition per quality, named after its longest GoP', async () => {
    const track = (name: string, height: number, keyframeSpacing: number) =>
      cmafTrack(name, { height, keyframeSpacing })
    const c = await cmafChain([
      track('a15', 540, 15),
      track('a60', 540, 60),
      track('b15', 1080, 15),
      track('b60', 1080, 60),
    ])
    await flush()
    expect(c.source.getRenditions().map((r) => [r.id, r.height, r.active])).toEqual([
      ['a60', 540, true],
      ['b60', 1080, false],
    ])
    void c.source.selectRenditions(['b60'])
    await flush()
    // The active rendition was deselected: into the other ladder, at its earliest switch point
    expect(c.requests().at(-2)).toMatch(/^subscribe b15 /)
  })
}
