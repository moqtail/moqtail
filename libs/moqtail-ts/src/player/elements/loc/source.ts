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

/** `LocSource`: a switching set source for LOC tracks (draft-ietf-moq-loc-05). */

import type { MoqtObject } from '@/model/data/object'
import { CodecString } from '@/model/property/codec_string'
import { findProperty } from '@/model/property/find'
import { Timescale } from '@/model/property/timescale'
import { Timestamp } from '@/model/property/timestamp'
import type { TrackProperty } from '@/model/property/track_property'
import type { Capabilities } from '@/player/element/define'
import type { CatalogTrack } from '@/player/element/interfaces'
import { packetOf, type Packet, type PadProperties } from '@/player/element/pad'
import { acceptedInitData, initOnObjects, objectInitData } from '@/player/element/source/init_data'
import { SwitchingSetSource } from '@/player/element/source/switching_set'
import { renditionId } from '@/player/util/track'
import { decoderConfigOf, locCapabilities } from './track'

/** Init data from the catalog or the config properties (MSF 12.2), timing from the LOC Timestamp. */
export class LocSource extends SwitchingSetSource {
  padCapabilities(role: string): Capabilities {
    return locCapabilities('out', role)
  }

  mediaProperties(track: CatalogTrack, trackProperties: TrackProperty[]): Partial<PadProperties> {
    // LOC 2.3.1.3: CODEC_STRING is the WebCodecs codec string
    // shortcut: an object-scope CODEC_STRING is ignored, read it in `packet` once a publisher changes codec mid-track
    const codec = findProperty(trackProperties, CodecString)?.codec ?? track.codec!
    const timescale = findProperty(trackProperties, Timescale)?.timescale
    return {
      decoderConfig: decoderConfigOf(track, codec),
      ...(timescale !== undefined && { timescale: Number(timescale) }),
    }
  }

  acceptedInitData(track: CatalogTrack, trackProperties: TrackProperty[]): Uint8Array | undefined {
    return acceptedInitData(track, trackProperties)
  }

  objectInitData(track: CatalogTrack, object: MoqtObject): Uint8Array | undefined {
    return objectInitData(track, object.properties)
  }

  initOnObjects(track: CatalogTrack): boolean {
    return initOnObjects(track)
  }

  mediaTime(_track: CatalogTrack, packet: Packet, carried: Partial<PadProperties>): number | undefined {
    const timestamp = findProperty(packet.properties, Timestamp)
    if (!timestamp) return undefined
    // The object's own TIMESCALE wins over the track's (LOC Table 1)
    const timescale = findProperty(packet.properties, Timescale)?.timescale ?? carried.timescale
    return timestamp.toMicros(timescale === undefined ? undefined : BigInt(timescale)) / 1e6
  }

  packet(track: CatalogTrack, object: MoqtObject, _initData: Uint8Array | undefined): Packet | undefined {
    return packetOf(renditionId(track, this.context.catalog), object, object.payload!)
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafChain, cmafTrack } = await import('@/player/testing/cmaf')
  const { mediaObject } = await import('@/player/testing/client')
  const { flush } = await import('@/player/testing/elements')
  const { UnknownTrackProperty } = await import('@/model/property/track_property')
  const { VideoConfig } = await import('@/model/property/video_config')
  const { Loc } = await import('./index')
  const locTrack = (extra: Partial<CatalogTrack> = {}) =>
    cmafTrack('v1', { packaging: 'loc', codec: 'avc3.64001f', width: 1280, height: 720, ...extra })

  test('the first properties carry the decoder config, the TIMESCALE and CODEC_STRING track properties, and the catalog’s init data', async () => {
    const trackProperties = [
      new VideoConfig(new Uint8Array([5])),
      new Timescale(90_000n),
      new CodecString('avc3.640028'),
    ].map((p) => new UnknownTrackProperty(p.toKeyValuePair()))
    const init = { type: 'track-property' as const, property: VideoConfig.TYPE }
    const c = await cmafChain([locTrack({ init })], {
      element: Loc,
      respond: () => ({ trackProperties }),
    })
    await flush()
    expect(c.sink.items()).toEqual([
      {
        type: 'properties',
        properties: {
          track: 'v1',
          live: true,
          decoderConfig: { codec: 'avc3.640028', codedWidth: 1280, codedHeight: 720 },
          timescale: 90_000,
          initData: new Uint8Array([5]),
        },
      },
    ])
  })

  test('packets keep the payload and properties, and group times come from the LOC Timestamp', async () => {
    const c = await cmafChain([locTrack()], { element: Loc })
    await flush()
    // Wall-clock µs, as without a TIMESCALE
    const at = (ms: number) => [new Timestamp(BigInt(1_700_000_000_000 + ms) * 1000n).toKeyValuePair()]
    const first = mediaObject(1, 0, at(0))
    c.client.last.push(first, mediaObject(1, 1, at(500)), mediaObject(2, 0, at(2000)))
    await flush()
    expect(c.trace()).toEqual(['props decoderConfig,live,track', '1.0', '1.1', '2.0', 'props groupDuration'])
    expect(c.sink.items()[1]).toMatchObject({ packet: { payload: first.payload, properties: first.properties } })
    expect(c.sink.items().at(-1)).toMatchObject({ properties: { groupDuration: 2 } })
  })
}
