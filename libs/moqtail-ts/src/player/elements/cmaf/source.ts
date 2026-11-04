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

import { SourceElement } from '../../element/base'
import type { ElementContext } from '../../element/context'
import type { CatalogTrack, MediaSourceElement, MediaSourceOptions, Rendition } from '../../element/interfaces'
import type { Pad } from '../../element/pad'
import { formatMsfUrl } from '../../msf_url'
import { Delivery } from './delivery'
import { Recovery } from './recovery'
import { bitrateOf, cmafCapabilities, renditionId, trackKey } from './track'

/**
 * Plays a CMAF switching set (`context.tracks`, at least one track) from its first track:
 * live with SUBSCRIBE from the next group, VOD with FETCH from the start.
 */
export class CmafSource extends SourceElement implements MediaSourceElement {
  readonly pad: Pad
  #tracks: readonly CatalogTrack[]
  /** Rendition ids ABR may choose from. */
  #selected: Set<string>
  readonly #recovery: Recovery

  constructor(
    context: ElementContext,
    readonly options: MediaSourceOptions,
  ) {
    super(context)
    const start = context.tracks[0]!
    this.#tracks = context.tracks
    this.#selected = new Set(context.tracks.map((t) => this.#id(t)))
    this.pad = this.createPad('src', cmafCapabilities(start.role ?? 'video'))
    const delivery = new Delivery(this.pad, context, options.requests)
    this.#recovery = new Recovery(
      context,
      delivery,
      {
        candidates: () => this.#tracks.filter((t) => this.#selected.has(this.#id(t))),
        giveUp: () => this.removePad(this.pad, true),
      },
      start,
    )
    this.emit('nomorepads')
    void this.#recovery.run()
  }

  getRenditions(): Rendition[] {
    const active = trackKey(this.#recovery.active)
    return this.#tracks.map((track) => ({
      id: this.#id(track),
      src: formatMsfUrl(this.context.catalog?.url ?? '', track.namespace, track.nameBytes),
      bitrate: bitrateOf(track),
      codec: track.codec ?? '',
      ...(track.width !== undefined && { width: track.width }),
      ...(track.height !== undefined && { height: track.height }),
      ...(track.framerate !== undefined && { frameRate: track.framerate }),
      active: trackKey(track) === active,
      selected: this.#selected.has(this.#id(track)),
    }))
  }

  // shortcut: rejects until switching lands in 3e.
  async switch(renditionId: string): Promise<void> {
    throw new Error(`switch(${renditionId}) isn't implemented yet`)
  }

  /** Stores which renditions ABR may choose from. */
  async selectRenditions(renditionIds: string[]): Promise<void> {
    this.#selected = new Set(renditionIds)
  }

  /** Replaces the switching set. New renditions start selected; the active track keeps playing. */
  setTracks(tracks: readonly CatalogTrack[]): void {
    const known = new Set(this.#tracks.map((t) => this.#id(t)))
    for (const track of tracks) if (!known.has(this.#id(track))) this.#selected.add(this.#id(track))
    this.#tracks = tracks
    const key = trackKey(this.#recovery.active)
    // The same track may come back with changes made in place, such as isLive turning false.
    const active = tracks.find((t) => trackKey(t) === key)
    if (active) this.#recovery.active = active
  }

  #id(track: CatalogTrack): string {
    return renditionId(track, this.context.catalog)
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafChain, cmafTrack, fakeCatalog } = await import('../../testing/cmaf')
  const { flush, track } = await import('../../testing/elements')

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
      ['v3', 0, false, true],
    ])
  })
}
