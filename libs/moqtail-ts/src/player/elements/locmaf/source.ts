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

/** `LocmafSource`: a switching set source for LOCMAF tracks, whose pad carries the CMAF they rebuild to. */

import { MoqtObject } from '@/model/data/object'
import type { CatalogTrack } from '@/player/element/interfaces'
import { CmafSource } from '@/player/elements/cmaf/source'
import { PlayerError } from '@/player/error'
import { trackKey } from '@/player/util/track'
import { Reference } from './chunk'
import { trackDefaults, type TrackDefaults } from './init'
import { canonicalChunk } from './moof'
import { LocmafError } from './object'

/** CMAF init data and timing, with each object rebuilt into its CMAF chunk (draft-einarsson-moq-locmaf-01 §16). */
export class LocmafSource extends CmafSource {
  readonly #references = new Map<string, Reference>()
  #defaults: { init: Uint8Array; defaults: TrackDefaults } | undefined
  /** The group a rejected object was last reported for. */
  #reported: string | undefined

  override packet(track: CatalogTrack, object: MoqtObject, initData: Uint8Array | undefined): MoqtObject | undefined {
    const key = trackKey(track)
    const reference = this.#references.get(key) ?? new Reference()
    this.#references.set(key, reference)
    try {
      if (!initData) throw new LocmafError('No CMAF Header to rebuild the chunk with')
      const defaults = this.#defaultsOf(initData)
      const decoded = reference.decode(object.location, object.payload!, defaults)
      const payload = decoded instanceof Uint8Array ? decoded : canonicalChunk(decoded, defaults)
      return MoqtObject.newWithPayload(
        object.fullTrackName,
        object.location,
        object.publisherPriority,
        object.objectForwardingPreference,
        object.subgroupId,
        object.properties,
        payload,
      )
    } catch (error) {
      if (!(error instanceof LocmafError)) throw error
      this.#report(key, object, error)
      return undefined
    }
  }

  /** The CMAF Header parameters of `init`, parsed once per header. */
  #defaultsOf(init: Uint8Array): TrackDefaults {
    if (this.#defaults?.init !== init) this.#defaults = { init, defaults: trackDefaults(init) }
    return this.#defaults.defaults
  }

  /** A warning for the first object of a group that can't be rebuilt; the rest of it fails the same way. */
  #report(track: string, object: MoqtObject, cause: LocmafError): void {
    const group = `${track}|${object.location.group}`
    if (this.#reported === group) return
    this.#reported = group
    const message = `LOCMAF object ${object.location.group}.${object.location.object} dropped: ${cause.message}`
    this.context.player.emit('warning', new PlayerError('MEDIA_FAILURE', message, { element: 'locmaf', cause }))
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafChain, cmafTrack } = await import('@/player/testing/cmaf')
  const { testObject } = await import('@/player/testing/client')
  const { flush } = await import('@/player/testing/elements')
  const { locmafVectors } = await import('../../../../test/locmaf')
  const { Locmaf } = await import('./index')
  type Cmaf = typeof import('@/player/elements/cmaf').Cmaf

  test('the pad carries CMAF chunks; an object that can’t be decoded leaves a gap and one warning', async () => {
    const [first, , third] = locmafVectors().find((v) => v.name === 'uniform')!.objects
    const init = { type: 'inline' as const, data: locmafVectors()[0]!.init }
    const track = cmafTrack('v1', { packaging: 'locmaf', locmafVersion: '0.3', init })
    const c = await cmafChain([track], { element: Locmaf as unknown as Cmaf })
    const warnings: unknown[] = []
    c.chain.host.player.on('warning', (warning) => warnings.push(warning))
    await flush()
    c.client.last.push(
      testObject('v1', 1, 0, first!.payload),
      testObject('v1', 1, 2, third!.payload),
      testObject('v1', 1, 3, third!.payload),
      testObject('v1', 2, 0, first!.payload),
    )
    await flush()
    // The group duration comes from the rebuilt chunks' tfdt
    expect(c.trace()).toEqual(['props initData,live,mimeType,track', '1.0', 'gap', '2.0', 'props groupDuration'])
    const packets = c.sink.items().filter((item) => item.type === 'packet')
    expect(packets.map((p) => p.object.payload)).toEqual([first!.canonical, first!.canonical])
    expect(warnings).toMatchObject([{ code: 'MEDIA_FAILURE', element: 'locmaf' }])
  })
}
