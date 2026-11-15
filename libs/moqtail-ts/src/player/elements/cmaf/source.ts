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

/** `CmafSource`: a switching set source for CMAF tracks (CMSF). */

import type { MoqtObject } from '@/model/data/object'
import type { TrackProperty } from '@/model/property/track_property'
import type { Capabilities } from '@/player/element/define'
import type { CatalogTrack } from '@/player/element/interfaces'
import { SwitchingSetSource } from '@/player/element/source/switching_set'
import { fragmentTime, mediaTimescale } from './bmff'
import { acceptedInitData, objectInitData } from './init_data'
import { cmafCapabilities, mimeTypeOf } from './track'

/** Init data from the catalog or the CMAF config properties (MSF 5.1.7), timing from the `moof`. */
export class CmafSource extends SwitchingSetSource {
  padCapabilities(role: string): Capabilities {
    return cmafCapabilities('out', role)
  }

  mimeType(track: CatalogTrack): string {
    return mimeTypeOf(track)
  }

  acceptedInitData(track: CatalogTrack, trackProperties: TrackProperty[]): Uint8Array | undefined {
    return acceptedInitData(track, trackProperties)
  }

  objectInitData(track: CatalogTrack, object: MoqtObject): Uint8Array | undefined {
    return objectInitData(track, object.properties)
  }

  initOnObjects(track: CatalogTrack): boolean {
    return track.init?.type === 'object-property'
  }

  mediaTime(track: CatalogTrack, object: MoqtObject, initData: Uint8Array | undefined): number | undefined {
    if (!object.payload) return undefined
    return fragmentTime(object.payload, track.timescale ?? (initData && mediaTimescale(initData)))
  }

  packet(_track: CatalogTrack, object: MoqtObject, _initData: Uint8Array | undefined): MoqtObject | undefined {
    return object
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafChain, cmafTrack, requested } = await import('@/player/testing/cmaf')
  const { mediaObject } = await import('@/player/testing/client')
  const { flush } = await import('@/player/testing/elements')
  const { UnknownTrackProperty } = await import('@/model/property/track_property')
  const { VideoConfig } = await import('@/model/property/video_config')
  const init = (byte: number) => new VideoConfig(new Uint8Array([byte]))

  test('init data from a track property, and a promised one that is missing fails the rendition', async () => {
    const property = { type: 'track-property' as const, property: VideoConfig.TYPE }
    const trackProperties = [new UnknownTrackProperty(init(5).toKeyValuePair())]
    const c = await cmafChain([cmafTrack('v1', { init: property }), cmafTrack('v2', { init: property })], {
      respond: (_, options) => (requested(options) === 'v2' ? { trackProperties } : {}),
    })
    await flush()
    // v1 had nothing, so the set moved on to v2 and unsubscribed v1
    expect(c.client.sent('unsubscribe')).toEqual([0n])
    expect(c.sink.items()).toMatchObject([{ properties: { track: 'v2', initData: new Uint8Array([5]) } }])
  })

  test('object-property init data: packets before it are dropped, a change writes properties again', async () => {
    const c = await cmafChain([cmafTrack('v1', { init: { type: 'object-property', property: VideoConfig.TYPE } })])
    await flush()
    const [a, b] = [init(1).toKeyValuePair(), init(2).toKeyValuePair()]
    c.client.last.push(mediaObject(1, 0), mediaObject(1, 1, [a]), mediaObject(1, 2, [a]), mediaObject(2, 0, [b]))
    await flush()
    expect(c.trace()).toEqual(['props live,mimeType,track', 'props initData', '1.1', '1.2', 'props initData', '2.0'])
    expect(c.sink.items()[4]).toMatchObject({ properties: { initData: new Uint8Array([2]) } })
  })
}
