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
 * Keeps every media track of the live test sources subscribed at the relay while a file runs,
 * on a client of its own. A relay that isn't subscribed to a track yet starts the first
 * subscriber mid-group and has nothing cached to fill from; warm, a test's SUBSCRIBE starts at
 * the next group and finds the cache it needs. `forward: false`, so none of it is sent here.
 */

import { afterAll, beforeAll } from 'vitest'
import { MOQtailClient } from '@/client/client'
import type { SubscribeOptions } from '@/client/types'
import { Tuple } from '@/model/common/tuple'
import { GroupOrder, LocationFilterType } from '@/model/control/constant'
import { RequestError } from '@/model/control/request_error'
import { FullTrackName } from '@/model/data/full_track_name'
import { LocationFilter } from '@/model/parameter/message/location_filter'
import { parseCatalog } from '@/player/elements/catalog/parse'
import { withRole } from '@/player/elements/catalog/selection'
import { RELAY, type TestSource } from './sources'

const options = (namespace: Tuple, name: string | Uint8Array, forward: boolean): SubscribeOptions => ({
  fullTrackName: FullTrackName.tryNew(namespace, name),
  priority: 255,
  groupOrder: GroupOrder.Original,
  forward,
  // The current group: its first object is a whole catalog.
  locationFilter: new LocationFilter(LocationFilterType.RelativeStart, 1n),
})

/** Registers `beforeAll`/`afterAll` that keep the live `sources`' media tracks subscribed. */
export function warmUp(...sources: TestSource[]): void {
  let client: MOQtailClient | undefined
  const subscribed: bigint[] = []
  beforeAll(async () => {
    client = new MOQtailClient({ url: RELAY })
    await client.init()
    for (const source of sources.filter((s) => s.live)) {
      const namespace = Tuple.fromUtf8Path(source.namespace)
      const catalog = await client.subscribe(options(namespace, 'catalog', true))
      if (catalog instanceof RequestError) throw new Error(`warm-up: catalog of ${source.namespace} rejected`)
      // A group's first object is a whole catalog, the rest deltas; status objects carry no payload.
      const reader = catalog.stream.getReader()
      let payload: Uint8Array | null | undefined
      while (!payload) {
        const { value, done } = await reader.read()
        if (done) throw new Error(`warm-up: catalog of ${source.namespace} ended`)
        if (value.location.object === 0n) payload = value.payload
      }
      reader.releaseLock()
      await client.unsubscribe(catalog.requestId)
      const { tracks } = parseCatalog(payload, { namespace, variables: {} }).catalog
      const media = tracks.map(withRole).filter((t) => t.role === 'video' || t.role === 'audio')
      const results = await Promise.all(media.map((t) => client!.subscribe(options(t.namespace, t.nameBytes, false))))
      const rejected = media.filter((_, i) => results[i] instanceof RequestError).map((t) => t.name)
      if (rejected.length > 0) throw new Error(`warm-up: ${rejected} of ${source.namespace} rejected`)
      for (const r of results) subscribed.push((r as Exclude<typeof r, RequestError>).requestId)
    }
  }, 20_000)
  afterAll(async () => {
    await Promise.all(subscribed.map((id) => client?.unsubscribe(id)))
    await client?.disconnect()
  })
}
