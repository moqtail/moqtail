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

import type { MOQtailClient } from '@/client/client'
import type { ObjectOrdering } from '@/client/types'
import { LocationFilterType } from '@/model/constants'
import type { MoqtObject } from '@/model/data/object'
import { locationFilter } from '@/model/parameters'
import { PlayerError } from '@/player/error'
import type { MsfSource } from '@/player/msf_url'
import {
  cancel,
  fetchFilter,
  mediaRequest,
  trackPropertiesOf,
  type Accepted,
  type ReadAhead,
  type TrackProperties,
} from './delivery'
import { merge } from './ranges'
import { isRequestError } from '@/model/messages'
import type { RequestError } from '@/model/messages'

/**
 * The request the player sends for a track URL no element accepts alone, so the probes can
 * read its track properties and objects. The selected source reads on from there: the request
 * is the one its first play would send.
 */
export class Preroll {
  /** False for a FETCH of the URL's first location range, true for a SUBSCRIBE from the next group. */
  readonly isLive: boolean
  readonly #client: MOQtailClient
  readonly #track: string
  readonly #request: Promise<RequestError | Accepted>
  readonly #objects: MoqtObject[] = []
  #accepted: Accepted | undefined
  #reader: ReadableStreamDefaultReader<MoqtObject> | undefined
  /** Handed off or cancelled: the request is no longer the preroll's. */
  #released = false

  constructor(client: MOQtailClient, source: MsfSource, ordering: ObjectOrdering | undefined) {
    this.#client = client
    this.#track = source.track

    // The first span `toSpans` gives a VOD track without a timeline
    const range = merge(source.locationRanges)[0]
    this.isLive = !range

    const common = mediaRequest(source.namespace, source.trackBytes, ordering)
    this.#request = range
      ? client.fetch({ ...common, locationFilter: fetchFilter(range.start, range.end) })
      : client.subscribe({ ...common, locationFilter: locationFilter(LocationFilterType.RelativeStart, 0n) })
  }

  /** The objects read so far, in order. */
  get objects(): readonly MoqtObject[] {
    return this.#objects
  }

  /** @throws PlayerError `UNSUPPORTED_SOURCE` when the request is rejected. */
  async trackProperties(): Promise<TrackProperties> {
    return trackPropertiesOf(await this.#accept())
  }

  /**
   * Reads the next object, kept for the source. Undefined once the stream ends.
   * @throws PlayerError `UNSUPPORTED_SOURCE` when the request is rejected.
   */
  async read(): Promise<MoqtObject | undefined> {
    const accepted = await this.#accept()

    this.#reader ??= accepted.stream.getReader()
    const { done, value } = await this.#reader.read()
    if (done) return undefined

    this.#objects.push(value)
    return value
  }

  /**
   * Gives the accepted request and the objects read to the source, and releases the stream for
   * the source's own reader. Undefined before the request is accepted.
   */
  handOff(): ReadAhead | undefined {
    const accepted = this.#accepted
    if (!accepted) return undefined

    this.#released = true
    this.#reader?.releaseLock()

    return { accepted, objects: this.#objects }
  }

  /** Ends the request, unless a source took it. */
  cancel(): void {
    if (this.#released) return

    this.#released = true
    void this.#reader?.cancel().catch(() => {}) // the session may be gone already

    void this.#request.then(
      (result) => cancel(this.#client, result),
      () => {}, // a request that failed is no request to end
    )
  }

  async #accept(): Promise<Accepted> {
    const result = await this.#request

    if (isRequestError(result)) {
      throw new PlayerError('UNSUPPORTED_SOURCE', `The request for track "${this.#track}" was rejected`, {
        cause: result,
      })
    }

    this.#accepted = result

    return result
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { FakeClient, mediaObject } = await import('@/player/testing/client')
  const { parseMsfUrl } = await import('@/player/msf_url')
  const { flush } = await import('@/player/testing/elements')
  const fake = new FakeClient()
  const client = fake as unknown as MOQtailClient

  test('the hand-off gives the objects read and an unlocked stream, and cancelling after it does nothing', async () => {
    const preroll = new Preroll(client, parseMsfUrl('moqt://relay.test/app#msf:ns--v1'), undefined)
    expect([preroll.isLive, preroll.handOff()]).toEqual([true, undefined])
    await flush()

    fake.last.push(mediaObject(1, 0), mediaObject(1, 1), mediaObject(1, 2))
    await preroll.read()
    await preroll.read()

    const { accepted, objects } = preroll.handOff()!
    preroll.cancel()
    expect(objects.map((o) => o.location.object)).toEqual([0n, 1n])

    const { value } = await accepted.stream.getReader().read()
    expect(value?.location.object).toBe(2n)
    expect(fake.sent('unsubscribe')).toEqual([])
  })
}
