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

/** `CmafSource`: one CMAF switching set on one source pad, and its renditions. */

import { SourceElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import type { CatalogTrack, MediaSourceElement, MediaSourceOptions, Rendition } from '@/player/element/interfaces'
import type { Pad } from '@/player/element/pad'
import { PlayerError } from '@/player/error'
import { formatMsfUrl } from '@/player/msf_url'
import { Delivery } from './delivery'
import type { Span } from './ranges'
import { closestBitrate, Recovery } from './recovery'
import { Switcher } from './switch'
import { bitrateOf, renditionId, trackKey } from '@/player/util/track'
import { cmafCapabilities } from './track'

/** What `Cmaf.init` worked out before the source starts: the URL ranges. */
export interface SourcePlan {
  spans: readonly Span[]
  /** The end of the last range in seconds, when known. */
  duration?: number
}

/**
 * Plays a CMAF switching set (`context.tracks`, at least one track) from its first track:
 * live with SUBSCRIBE from the next group, VOD with FETCH from the start, or the URL ranges.
 */
export class CmafSource extends SourceElement implements MediaSourceElement {
  readonly pad: Pad
  #tracks: readonly CatalogTrack[]
  /** Rendition ids ABR may choose from. */
  #selected: Set<string>
  readonly delivery: Delivery
  readonly recovery: Recovery
  readonly #switcher: Switcher

  constructor(
    context: ElementContext,
    readonly options: MediaSourceOptions,
    plan: SourcePlan = { spans: [] },
  ) {
    super(context)
    const start = context.tracks[0]!
    this.#tracks = context.tracks
    this.#selected = new Set(context.tracks.map((t) => this.#id(t)))
    this.pad = this.createPad('src', cmafCapabilities(start.role ?? 'video'))
    this.delivery = new Delivery(this.pad, context, options.requests)
    this.delivery.duration = plan.duration
    this.recovery = new Recovery(
      context,
      this.delivery,
      {
        tracks: () => this.#tracks,
        candidates: () => this.#tracks.filter((t) => this.#selected.has(this.#id(t))),
        giveUp: () => this.removePad(this.pad, true),
      },
      start,
      plan,
    )
    const { delivery, recovery } = this
    this.#switcher = new Switcher({ context, options, delivery, recovery, tracks: () => this.#tracks })
    this.delivery.onTrackStart = (track, changed) => this.#trackStarted(track, changed)
    this.pad.onEvent((event) => {
      if (event.type !== 'seek') return false
      this.recovery.seek(event.mediaTime)
      return true
    })
    this.emit('nomorepads')
    void this.recovery.run()
  }

  getRenditions(): Rendition[] {
    return this.#tracks.map((track) => this.#rendition(track))
  }

  /** Switches at the next switch point; resolves once the new track's first packet is written. */
  async switch(renditionId: string): Promise<void> {
    const track = this.#tracks.find((t) => this.#id(t) === renditionId)
    if (!track) throw new PlayerError('UNSUPPORTED_SOURCE', `Unknown rendition ${renditionId}`, { element: 'cmaf' })
    return this.#switcher.switch(track)
  }

  /**
   * Sets which renditions ABR may choose from. A deselected active rendition switches to the
   * closest selected bitrate; an empty selection stops requesting until a later one.
   */
  async selectRenditions(renditionIds: string[]): Promise<void> {
    this.#selected = new Set(renditionIds)
    const selected = this.#tracks.filter((t) => this.#selected.has(this.#id(t)))
    if (selected.length === 0) return this.recovery.pause()
    this.recovery.resume()
    const active = this.recovery.active
    if (!this.#selected.has(this.#id(active))) await this.switch(this.#id(closestBitrate(selected, bitrateOf(active))!))
  }

  /**
   * Replaces the switching set. New renditions start selected, unless exactly one is selected.
   * A removed active track switches to the closest selected bitrate; an empty set removes the pad.
   */
  setTracks(tracks: readonly CatalogTrack[]): void {
    const known = new Set(this.#tracks.map((t) => this.#id(t)))
    const keepSelection = this.#selected.size === 1
    for (const track of tracks) if (!known.has(this.#id(track)) && !keepSelection) this.#selected.add(this.#id(track))
    this.#tracks = tracks
    if (tracks.length === 0) {
      this.recovery.pause()
      if (this.pads.includes(this.pad)) this.removePad(this.pad)
      return
    }
    const key = trackKey(this.recovery.active)
    // The same track may come back with changes made in place, such as isLive turning false.
    const active = tracks.find((t) => trackKey(t) === key)
    if (active) {
      this.recovery.active = active
      return
    }
    const selected = tracks.filter((t) => this.#selected.has(this.#id(t)))
    const next = closestBitrate(selected.length > 0 ? selected : tracks, bitrateOf(this.recovery.active))!
    void this.switch(this.#id(next)).catch(() => {})
  }

  #trackStarted(track: CatalogTrack, changed: boolean): void {
    if (trackKey(track) !== trackKey(this.recovery.active)) this.recovery.active = track
    this.recovery.dropHandover(track)
    this.#switcher.started(track)
    if (changed) this.context.player.emit('renditionchange', this, this.#rendition(track))
  }

  #rendition(track: CatalogTrack): Rendition {
    return {
      id: this.#id(track),
      src: formatMsfUrl(this.context.catalog?.url ?? '', track.namespace, track.nameBytes),
      bitrate: bitrateOf(track),
      codec: track.codec ?? '',
      ...(track.width !== undefined && { width: track.width }),
      ...(track.height !== undefined && { height: track.height }),
      ...(track.framerate !== undefined && { frameRate: track.framerate }),
      active: trackKey(track) === trackKey(this.recovery.active),
      selected: this.#selected.has(this.#id(track)),
    }
  }

  #id(track: CatalogTrack): string {
    return renditionId(track, this.context.catalog)
  }
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
      // Exactly one rendition was selected: a new one doesn't join the selection.
      ['v3', 0, false, false],
    ])
  })
}
