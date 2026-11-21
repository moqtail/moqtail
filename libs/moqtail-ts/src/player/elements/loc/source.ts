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
import type { TrackProperty } from '@/model/property/track_property'
import type { Capabilities } from '@/player/element/define'
import type { CatalogTrack } from '@/player/element/interfaces'
import { packetOf, type Packet, type PadProperties } from '@/player/element/pad'
import { acceptedInitData, initOnObjects, objectInitData } from '@/player/element/source/init_data'
import { SwitchingSetSource } from '@/player/element/source/switching_set'
import { renditionId } from '@/player/util/track'
import { decoderConfigOf, locCapabilities, timestampMicros } from './track'

/** Init data from the catalog or the config properties (MSF 12.2), timing from the LOC Timestamp. */
export class LocSource extends SwitchingSetSource {
  padCapabilities(role: string): Capabilities {
    return locCapabilities('out', role)
  }

  mediaProperties(track: CatalogTrack, trackProperties: TrackProperty[]): Partial<PadProperties> {
    // LOC 2.3.1.3: CODEC_STRING is the WebCodecs codec string
    // shortcut: an object-scope CODEC_STRING is ignored, read it in `packet` once a publisher changes codec mid-track
    const codec = findProperty(trackProperties, CodecString)?.codec ?? track.codec!
    // LOC 2.3.1.2: only TIMESCALE sets it; without it Timestamps are epoch µs, whatever the catalog says
    const timescale = findProperty(trackProperties, Timescale)?.timescale
    return {
      decoderConfig: decoderConfigOf(track, codec),
      // Always present, so a switch to a track without one clears the previous track's
      timescale: timescale === undefined ? undefined : Number(timescale),
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
    const micros = timestampMicros(packet.properties, carried.timescale)
    return micros === undefined ? undefined : micros / 1e6
  }

  packet(track: CatalogTrack, object: MoqtObject, _initData: Uint8Array | undefined): Packet | undefined {
    return packetOf(renditionId(track, this.context.catalog), object, object.payload!)
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { cmafChain, cmafTrack, requested } = await import('@/player/testing/cmaf')
  const { mediaObject } = await import('@/player/testing/client')
  const { flush } = await import('@/player/testing/elements')
  const { UnknownTrackProperty } = await import('@/model/property/track_property')
  const { Timestamp } = await import('@/model/property/timestamp')
  const { VideoConfig } = await import('@/model/property/video_config')
  const { Loc } = await import('./index')
  const locTrack = (extra: Partial<CatalogTrack> = {}, name = 'v1') =>
    cmafTrack(name, { packaging: 'loc', codec: 'avc3.64001f', width: 1280, height: 720, ...extra })

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
    const c = await cmafChain([locTrack({ timescale: 90_000 })], { element: Loc })
    await flush()

    // Wall-clock µs, as without a TIMESCALE track property: the catalog's timescale isn't LOC's
    const at = (ms: number) => [new Timestamp(BigInt(1_700_000_000_000 + ms) * 1000n).toKeyValuePair()]
    const first = mediaObject(1, 0, at(0))
    c.client.last.push(first, mediaObject(1, 1, at(500)), mediaObject(2, 0, at(2000)))
    await flush()

    expect(c.trace()).toEqual(['props decoderConfig,live,track', '1.0', '1.1', '2.0', 'props groupDuration'])
    expect(c.sink.items()[1]).toMatchObject({ packet: { payload: first.payload, properties: first.properties } })
    expect(c.sink.items().at(-1)).toMatchObject({ properties: { groupDuration: 2 } })
  })

  const ticks = (timestamp: bigint, timescale?: bigint) => [
    new Timestamp(timestamp).toKeyValuePair(),
    ...(timescale === undefined ? [] : [new Timescale(timescale).toKeyValuePair()]),
  ]

  test('an object’s TIMESCALE wins over the track’s', async () => {
    const c = await cmafChain([locTrack()], { element: Loc })
    const time = (properties: ReturnType<typeof ticks>, timescale?: number) =>
      c.source.mediaTime(locTrack(), packetOf('v1', mediaObject(1, 0, properties), new Uint8Array()), { timescale })

    expect(time(ticks(3000n, 1000n), 90_000)).toBe(3)
    expect(time(ticks(180_000n), 90_000)).toBe(2)
    expect(time(ticks(2_000_000n))).toBe(2)
  })

  test('a switch to a track without a timescale clears the previous track’s', async () => {
    const trackProperties = [new UnknownTrackProperty(new Timescale(90_000n).toKeyValuePair())]
    const c = await cmafChain([locTrack(), locTrack({}, 'v2')], {
      element: Loc,
      respond: (_, options) => (requested(options) === 'v1' ? { trackProperties } : {}),
    })
    await flush()

    c.client.last.push(mediaObject(1, 0, ticks(90_000n)))
    await flush()

    const switched = c.source.switch('v2')
    await flush()

    const [v1, v2] = c.client.requests
    v1!.push(mediaObject(2, 0, ticks(180_000n))) // past the switch point
    await flush()
    v2!.push(mediaObject(2, 0, ticks(2_000_000n), 'v2'))

    await switched

    expect(c.trace().slice(-2)).toEqual(['props timescale,track', '2.0'])
    expect(c.sink.items().at(-2)).toStrictEqual({
      type: 'properties',
      properties: { track: 'v2', timescale: undefined },
    })
  })
}
