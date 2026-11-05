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

/** Doubles for CMAF source tests. Never exported from the player entry. */

import { Location } from '@/model/common/location'
import { Tuple, TupleField } from '@/model/common/tuple'
import { ObjectForwardingPreference, ObjectStatus } from '@/model/data/constant'
import { FullTrackName } from '@/model/data/full_track_name'
import { MoqtObject } from '@/model/data/object'
import { Cmaf } from '@/player/elements/cmaf'
import type { CmafSource } from '@/player/elements/cmaf/source'
import type { SubscribeOptions, SubscribeUpdateOptions, SwitchOptions } from '@/client/types'
import { LocationFilterType } from '@/model/control/constant'
import { FillParameters } from '@/model/parameter/message/fill_parameters'
import type { LocationFilter } from '@/model/parameter/message/location_filter'
import type { CatalogReader, CatalogTrack, MediaSourceOptions, TimelineReader } from '@/player/element/interfaces'
import { fragment } from './bmff'
import { FakeClient } from './client'
import { collectingSink, testChain, testSource, track, type CollectingSink } from './elements'

/** A CMAF video track in namespace `ns`. */
export function cmafTrack(name: string, extra: Partial<CatalogTrack> = {}): CatalogTrack {
  return { ...track(name), role: 'video', codec: 'avc1.64001f', ...extra }
}

/** A catalog reader the test edits; emit the player's `catalog` event after editing. */
export function fakeCatalog(tracks: CatalogTrack[] = [], timeline?: TimelineReader) {
  return {
    url: 'moqt://relay.test/app',
    namespace: new Tuple([TupleField.fromUtf8('ns')]),
    tracks,
    isComplete: false as boolean, // tests flip it
    timeline: async (): Promise<TimelineReader | undefined> => timeline,
  } satisfies CatalogReader
}

/** The track name a request was sent for. */
export const requested = (options: { fullTrackName: FullTrackName }) =>
  new TextDecoder().decode(options.fullTrackName.name)

/** An end of track status object. */
export function endOfTrack(group: number, object = 0): MoqtObject {
  const name = FullTrackName.tryNew('ns', 'v1')
  const location = new Location(group, object)
  return MoqtObject.newWithStatus(
    name,
    location,
    0,
    ObjectForwardingPreference.Subgroup,
    0,
    null,
    ObjectStatus.EndOfTrack,
  )
}

/**
 * A `CmafSource` loaded for `tracks` as the catalog would, over a `FakeClient`, with its
 * pad connected to a collecting sink.
 */
export async function cmafChain(
  tracks: CatalogTrack[],
  options: {
    catalog?: CatalogReader
    cmaf?: MediaSourceOptions
    reconnectAttempts?: number
    respond?: FakeClient['respond']
    ordered?: boolean
  } = {},
) {
  const client = () => {
    const created = new FakeClient()
    created.ordered = options.ordered ?? false
    if (options.respond) created.respond = options.respond
    return created
  }
  const clients = [client()]
  const { chain, failures } = testChain(
    () => [],
    () => clients.at(-1),
  )
  if (options.reconnectAttempts)
    Object.assign(chain.host.player.options, { client: { reconnectAttempts: options.reconnectAttempts } })
  let source!: CmafSource
  let sink!: CollectingSink
  chain.addSource(
    testSource({
      script: async (parent) => {
        const { catalog, cmaf = {} } = options
        source = await parent.ctx.load(Cmaf, { tracks, options: cmaf, ...(catalog && { catalog }) })
        parent.emit('nomorepads')
      },
    }),
    {},
  )
  chain.addSink(collectingSink({ setup: (k) => (sink = k) }))
  await chain.init(new AbortController().signal)
  const player = chain.host.player
  return {
    chain,
    source,
    sink,
    failures,
    get client(): FakeClient {
      return clients.at(-1)!
    },
    /** Loses the session; `reconnected()` then brings a new client. */
    lose() {
      clients.at(-1)!.drop()
      player.emit('reconnecting', 1)
    },
    reconnected() {
      clients.push(client())
      player.emit('reconnected')
    },
    /** What the sink received, in short: `props <keys>`, `<group>.<object>`, `gap`, `eos`. */
    trace(): string[] {
      // The first pad's log, kept after the pad is removed.
      return (sink.log.values().next().value ?? []).map((item) => {
        if (item.type === 'packet') return `${item.object.location.group}.${item.object.location.object}`
        if (item.type === 'properties') return `props ${Object.keys(item.properties).sort().join(',')}`
        if (item.event.type !== 'discontinuity') return item.event.type
        return item.event.failed ? `${item.event.reason} failed` : item.event.reason
      })
    },
    /** Sends a `seek` upstream, as the sink does. */
    seek(mediaTime: number) {
      sink.pads[0]!.sendEvent({ type: 'seek', mediaTime })
    },
    /** The requests sent on the current client, in short. */
    requests(): string[] {
      return clients.at(-1)!.calls.map(describeCall)
    },
  }
}

/** A request in short: `subscribe v1 next`, `fetch v1 2.0-4`, `update 0 3.0-4.14`, `switch 0 v2 5.0- fill`. */
function describeCall({ method, args }: { method: string; args: unknown }): string {
  if (method === 'unsubscribe' || method === 'fetchCancel') return `${method} ${args}`
  if (method === 'subscribeUpdate') {
    const update = args as SubscribeUpdateOptions
    return `update ${update.subscriptionRequestId} ${describeFilter(update.locationFilter)}`
  }
  const options = (method === 'switch' ? (args as SwitchOptions).newSubscribeOptions : args) as SubscribeOptions
  const fill = options.parameters?.find((p): p is FillParameters => p instanceof FillParameters)
  const filled = fill && ` fill${fill.parameters.map((f) => ` ${describeFilter(f as LocationFilter)}`).join('')}`
  const from = method === 'switch' ? ` ${(args as SwitchOptions).switchFromRequestId}` : ''
  return `${method}${from} ${requested(options)} ${describeFilter(options.locationFilter)}${filled ?? ''}`
}

function describeFilter(filter: LocationFilter | undefined): string {
  if (!filter) return 'all'
  const { filterType, startGroup: g, startObject: o, endGroup, endObject } = filter
  if (filterType === LocationFilterType.RelativeStart) return g ? `next-${g}` : 'next'
  if (filterType === LocationFilterType.AbsoluteStart) return `${g}.${o}-`
  if (filterType === LocationFilterType.AbsoluteStartGroupEnd) return `${g}.${o}-${endGroup}`
  return `${g}.${o}-${endGroup}.${endObject}`
}

/** A media object of `track` whose payload is a fragment at `time` seconds (timescale 1000). */
export function timedObject(group: number, time: number, track = 'v1', object = 0): MoqtObject {
  return MoqtObject.newWithPayload(
    FullTrackName.tryNew('ns', track),
    new Location(group, object),
    0,
    ObjectForwardingPreference.Subgroup,
    0,
    null,
    fragment(time * 1000),
  )
}

/** A timeline where group `1 + n` starts at `n × 2` seconds, and the live edge is at `edgeMs`. */
export function twoSecondTimeline(edgeMs?: number): TimelineReader {
  return {
    timeToLocation: (ms) => new Location(1 + Math.floor(ms / 2000), 0),
    locationToTime: (location) => Number(location.group - 1n) * 2000,
    wallclockToLocation: (ms) => new Location(1 + Math.floor(ms / 2000), 0),
    range: edgeMs === undefined ? undefined : { start: 0, end: edgeMs },
  }
}
