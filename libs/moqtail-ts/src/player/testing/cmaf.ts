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

import { Location } from '../../model/common/location'
import { Tuple, TupleField } from '../../model/common/tuple'
import { ObjectForwardingPreference, ObjectStatus } from '../../model/data/constant'
import { FullTrackName } from '../../model/data/full_track_name'
import { MoqtObject } from '../../model/data/object'
import { Emitter } from '../../util/emitter'
import { Cmaf } from '../elements/cmaf'
import type { CmafSource } from '../elements/cmaf/source'
import type { CatalogReader, CatalogTrack, MediaSourceOptions } from '../element/interfaces'
import { FakeClient } from './client'
import { collectingSink, testChain, testSource, track, type CollectingSink } from './elements'

/** A CMAF video track in namespace `ns`. */
export function cmafTrack(name: string, extra: Partial<CatalogTrack> = {}): CatalogTrack {
  return { ...track(name), role: 'video', codec: 'avc1.64001f', ...extra }
}

/** A catalog reader the test edits; emit `change` after editing. */
export function fakeCatalog(tracks: CatalogTrack[] = []) {
  return Object.assign(new Emitter<{ change: () => void }>(), {
    url: 'moqt://relay.test/app',
    namespace: new Tuple([TupleField.fromUtf8('ns')]),
    tracks,
    isComplete: false,
    timeline: async () => undefined,
  }) satisfies CatalogReader
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
  } = {},
) {
  const client = () => {
    const created = new FakeClient()
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
      return sink.items().map((item) => {
        if (item.type === 'packet') return `${item.object.location.group}.${item.object.location.object}`
        if (item.type === 'properties') return `props ${Object.keys(item.properties).sort().join(',')}`
        return item.event.type === 'discontinuity' ? item.event.reason : item.event.type
      })
    },
  }
}
