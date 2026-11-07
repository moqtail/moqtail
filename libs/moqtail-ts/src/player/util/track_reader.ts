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
 * `TrackReader`: the join, ordering, and decompression shared by the catalog and the
 * timeline elements (MSF 5, 7.3, 8.4). Object 0 of a group is independent, later objects are
 * incremental and applied in object ID order. A missing, failed, or skipped object makes
 * the group stale until the next independent object; without one within the wait, the
 * reader resubscribes.
 */

import type { MOQtailClient } from '@/client/client'
import type { SubscribeResult } from '@/client/types'
import type { Location } from '@/model/common/location'
import type { Tuple } from '@/model/common/tuple'
import { GroupOrder, LocationFilterType } from '@/model/control/constant'
import { RequestError } from '@/model/control/request_error'
import type { SubscribeOk } from '@/model/control/subscribe_ok'
import { FullTrackName } from '@/model/data/full_track_name'
import type { MoqtObject } from '@/model/data/object'
import { FillParameters } from '@/model/parameter/message/fill_parameters'
import { LocationFilter } from '@/model/parameter/message/location_filter'
import { findProperty } from '@/model/property/find'
import { DEFAULT_MAX_CATALOG_BYTES } from '@/player/constants'
import type { ElementContext } from '@/player/element/context'
import { PlayerError } from '@/player/error'
import { backoffDelay, sleep } from '@/player/util/async'
import {
  decompress,
  isSupportedCompression,
  MsfCompressionObjectProperty,
  MsfCompressionTrackProperty,
} from './compression'

/** How long a missing object, or a stale group's next independent object, is waited for. */
const ORDERING_WINDOW_MS = 2000

export interface TrackReaderOptions {
  track: { namespace: Tuple; name: Uint8Array }
  priority: number
  /** false: one independent object per group, later objects are ignored (MSF 8.4). */
  incremental: boolean
  /** Limit for a decompressed payload, `opts.maxCatalogBytes` by default. */
  maxBytes?: number
  /** Throwing marks the group stale. `object` still holds the compressed payload. */
  onIndependent(payload: Uint8Array, location: Location, object: MoqtObject): void
  /** Throwing marks the group stale. */
  onIncremental(payload: Uint8Array, location: Location, object: MoqtObject): void
  /** The group went stale, or the track can't be read at all, with why. */
  onStale(reason: Error): void
}

type Subscription = { client: MOQtailClient; result: SubscribeResult }

export class TrackReader {
  readonly #context: ElementContext
  readonly #options: TrackReaderOptions
  readonly #maxBytes: number
  readonly #abort = new AbortController()
  readonly #timers = new Set<ReturnType<typeof setTimeout>>()
  #current: Subscription | undefined
  /** The latest group whose object 0 arrived, the next object ID it expects, and whether it's stale. */
  #group: { id: bigint; next: bigint; stale: boolean } | undefined
  /** Incremental objects that arrived ahead of their predecessor, with their arrival time. */
  #waiting: { object: MoqtObject; at: number }[] = []
  /** Objects are handled one at a time, since decompression is async. */
  #queue = Promise.resolve()
  #gapCheckScheduled = false

  constructor(context: ElementContext, options: TrackReaderOptions) {
    this.#context = context
    this.#options = options
    this.#maxBytes = options.maxBytes ?? context.player.options.maxCatalogBytes ?? DEFAULT_MAX_CATALOG_BYTES
  }

  get subscribeOk(): SubscribeOk | undefined {
    return this.#current?.result.subscribeOk
  }

  /**
   * Subscribes from the latest group, object 0 included (moqt-22 §3.4). Resolves on
   * SUBSCRIBE_OK; later losses resubscribe on their own until `stop()`.
   * @throws PlayerError when the subscription is rejected or can't be sent.
   */
  async start(): Promise<void> {
    const client = this.#context.client
    const { namespace, name } = this.#options.track
    let result: RequestError | SubscribeResult
    try {
      result = await client.subscribe({
        fullTrackName: FullTrackName.tryNew(namespace, name),
        priority: this.#options.priority,
        groupOrder: GroupOrder.Original,
        forward: true,
        // The reader orders its own objects
        ordering: 'none',
        // The next object live, and the latest group from object 0 as fill (no Joining FETCH in moqt-22)
        locationFilter: new LocationFilter(LocationFilterType.NextObject),
        parameters: [new FillParameters([new LocationFilter(LocationFilterType.RelativeStart, 1n)])],
      })
    } catch (cause) {
      throw new PlayerError('UNSUPPORTED_SOURCE', 'Subscribing failed', { cause })
    }
    if (result instanceof RequestError) {
      throw new PlayerError('UNSUPPORTED_SOURCE', `Subscribe rejected: ${result.reasonPhrase.phrase}`)
    }
    if (this.#abort.signal.aborted) {
      void unsubscribe({ client, result })
      return
    }
    this.#current = { client, result }
    this.#waiting = []
    const algorithm = findProperty(result.subscribeOk.trackProperties, MsfCompressionTrackProperty)?.algorithm
    if (algorithm !== undefined && !isSupportedCompression(algorithm)) {
      await this.stop()
      this.#options.onStale(new Error(`Unsupported MSF compression algorithm ${algorithm} (MSF 12.1.1)`))
      return
    }
    void this.#read(this.#current)
  }

  async stop(): Promise<void> {
    this.#abort.abort()
    for (const timer of this.#timers) clearTimeout(timer)
    const current = this.#current
    this.#current = undefined
    if (current) await unsubscribe(current)
  }

  async #read(current: Subscription): Promise<void> {
    const reader = current.result.stream.getReader()
    try {
      for (let read = await reader.read(); !read.done; read = await reader.read()) {
        const object = read.value
        const at = performance.now()
        this.#queue = this.#queue.then(() => this.#arrived(object, at))
      }
    } catch {
      // Reset, or the session was lost: resubscribed below
    }
    if (this.#current === current) void this.#resubscribe()
  }

  /** Ends the current subscription and subscribes again with the same filters, retrying with backoff. */
  async #resubscribe(): Promise<void> {
    const current = this.#current
    this.#current = undefined
    if (current) void unsubscribe(current)
    for (let attempt = 0; !this.#abort.signal.aborted; attempt++) {
      try {
        return await this.start()
      } catch {
        await sleep(backoffDelay(attempt), this.#abort.signal).catch(() => {})
      }
    }
  }

  async #arrived(object: MoqtObject, at: number): Promise<void> {
    const { group: groupId, object: objectId } = object.location
    const group = this.#group
    if (this.#abort.signal.aborted || !object.payload) return
    if (group && groupId < group.id) return // MSF 5: older than the latest independent object
    if (objectId === 0n) {
      // The same group again only matters to a stale group, after a resubscribe
      if (group && groupId === group.id && !group.stale) return
      this.#group = { id: groupId, next: 1n, stale: false }
      this.#waiting = this.#waiting.filter((w) => w.object.location.group >= groupId)
      await this.#apply(object, 'onIndependent')
    } else if (this.#options.incremental) {
      this.#waiting.push({ object, at })
    }
    await this.#drain()
  }

  /** Applies waiting objects that are next in the latest group, then checks for a gap. */
  async #drain(): Promise<void> {
    for (;;) {
      const group = this.#group
      if (!group || group.stale) return
      this.#waiting = this.#waiting.filter(
        (w) => w.object.location.group > group.id || w.object.location.object >= group.next,
      )
      const index = this.#waiting.findIndex(
        (w) => w.object.location.group === group.id && w.object.location.object === group.next,
      )
      if (index < 0) break
      const { object } = this.#waiting.splice(index, 1)[0]!
      group.next++
      await this.#apply(object, 'onIncremental')
    }
    this.#checkGap()
  }

  /** A later object has waited past the window for its predecessor. Timers only run late, so elapsed time decides. */
  #checkGap(): void {
    const group = this.#group
    if (!group || group.stale) return
    const ahead = this.#waiting.filter((w) => w.object.location.group === group.id)
    if (ahead.length === 0) return
    const waited = performance.now() - Math.min(...ahead.map((w) => w.at))
    if (waited >= ORDERING_WINDOW_MS) {
      this.#markStale(new Error(`Object ${group.id}.${group.next} is missing`))
      return
    }
    if (this.#gapCheckScheduled) return
    this.#gapCheckScheduled = true
    this.#schedule(ORDERING_WINDOW_MS - waited, () => {
      this.#gapCheckScheduled = false
      this.#checkGap()
    })
  }

  async #apply(object: MoqtObject, handler: 'onIndependent' | 'onIncremental'): Promise<void> {
    const trackAlgorithm = findProperty(this.subscribeOk?.trackProperties, MsfCompressionTrackProperty)?.algorithm
    const algorithm = trackAlgorithm ?? findProperty(object.properties, MsfCompressionObjectProperty)?.algorithm ?? 0
    try {
      const payload = await decompress(object.payload!, algorithm, this.#maxBytes)
      this.#options[handler](payload, object.location, object)
    } catch (error) {
      this.#markStale(error instanceof Error ? error : new Error(String(error)))
    }
  }

  /** Later objects of the group are ignored; without a new independent object in time, resubscribe. */
  #markStale(reason: Error): void {
    const group = this.#group
    if (!group || group.stale) return
    group.stale = true
    this.#waiting = this.#waiting.filter((w) => w.object.location.group > group.id)
    this.#options.onStale(reason)
    this.#schedule(ORDERING_WINDOW_MS, () => {
      if (this.#group === group) void this.#resubscribe()
    })
  }

  /** Queues `run` behind the objects being handled, after `ms`. */
  #schedule(ms: number, run: () => void): void {
    if (this.#abort.signal.aborted) return
    const timer = setTimeout(() => {
      this.#timers.delete(timer)
      this.#queue = this.#queue.then(run)
    }, ms)
    this.#timers.add(timer)
  }
}

/** Ends a subscription this reader no longer reads; the client may be gone already. */
async function unsubscribe({ client, result }: Subscription): Promise<void> {
  await client.unsubscribe(result.requestId).catch(() => {})
}

if (import.meta.vitest) {
  const { describe, test, expect, vi, afterEach } = import.meta.vitest
  const { FakeClient, gzip, refuse, testObject } = await import('@/player/testing/client')
  const { KeyValuePair } = await import('@/model/common/pair')
  const { Tuple } = await import('@/model/common/tuple')
  const { flush, settle } = await import('@/player/testing/elements')

  const text = (s: string) => new TextEncoder().encode(s)
  const object = (group: number, id: number, payload: Uint8Array | string, algorithm?: number) =>
    testObject(
      'v1',
      group,
      id,
      typeof payload === 'string' ? text(payload) : payload,
      algorithm === undefined ? null : [new MsfCompressionObjectProperty(algorithm).toKeyValuePair()],
    )

  /** A reader over a fake client that logs what it applied as `i <group>.<object> <payload>`, `d ...`, `stale`. */
  function setup(incremental = true) {
    const client = new FakeClient()
    const log: string[] = []
    const entry = (kind: string) => (payload: Uint8Array, at: Location) =>
      log.push(`${kind} ${at.group}.${at.object} ${new TextDecoder().decode(payload)}`)
    const context = { client, signal: new AbortController().signal } as unknown as ElementContext
    const reader = new TrackReader(context, {
      track: { namespace: Tuple.fromUtf8Path('ns'), name: text('catalog') },
      priority: 0,
      incremental,
      maxBytes: 1 << 20,
      onIndependent: entry('i'),
      onIncremental: (payload, at) => {
        if (new TextDecoder().decode(payload) === 'bad') throw new Error('bad delta')
        entry('d')(payload, at)
      },
      onStale: (reason) => log.push(`stale ${reason.message}`),
    })
    return { client, log, reader, settle }
  }

  afterEach(() => void vi.useRealTimers())

  describe('TrackReader', () => {
    test('joins with NextObject and a fill of the latest group, then applies fill and live in order', async () => {
      const { client, log, reader, settle } = setup()
      await reader.start()
      const sent = client.sent<import('@/client/types').SubscribeOptions>('subscribe')[0]!
      expect([sent.priority, sent.locationFilter?.filterType]).toEqual([0, LocationFilterType.NextObject])
      const fill = sent.parameters?.[0] as FillParameters
      expect((fill.parameters[0] as LocationFilter).filterType).toBe(LocationFilterType.RelativeStart)
      expect((fill.parameters[0] as LocationFilter).startGroup).toBe(1n)

      // An old group's object, a delta ahead of its predecessor, and the next group
      client.last.push(object(5, 0, 'a'), object(5, 2, 'c'), object(4, 1, 'old'), object(5, 1, 'b'), object(6, 1, 'x'))
      client.last.push(object(6, 0, 'n'), object(5, 3, 'late'))
      await settle()
      expect(log).toEqual(['i 5.0 a', 'd 5.1 b', 'd 5.2 c', 'i 6.0 n', 'd 6.1 x'])
      await reader.stop()
    })

    test('a gap past the window, or a failed delta, makes the group stale; no new group resubscribes', async () => {
      vi.useFakeTimers({ toFake: ['setTimeout', 'performance'] })
      const { client, log, reader } = setup()
      await reader.start()
      client.last.push(object(1, 0, 'a'), object(1, 2, 'c'))
      await vi.advanceTimersByTimeAsync(ORDERING_WINDOW_MS)
      expect(log).toEqual(['i 1.0 a', 'stale Object 1.1 is missing'])
      client.last.push(object(1, 1, 'b'))
      await vi.advanceTimersByTimeAsync(ORDERING_WINDOW_MS)
      expect(client.sent('subscribe')).toHaveLength(2)
      // The fill brings the stale group again, which now applies
      client.last.push(object(1, 0, 'a'), object(1, 1, 'bad'), object(1, 2, 'c'))
      await vi.advanceTimersByTimeAsync(0)
      expect(log.slice(2)).toEqual(['i 1.0 a', 'stale bad delta'])
      await reader.stop()
    })

    test('nothing applies before an independent object; deltas are ignored without incremental', async () => {
      const { client, log, reader, settle } = setup(false)
      await reader.start()
      client.last.push(object(3, 1, 'd'), object(3, 0, 'i'), object(3, 1, 'd'))
      await settle()
      expect(log).toEqual(['i 3.0 i'])
      await reader.stop()
    })

    test('decompresses by track property, else by object property; an unknown algorithm is stale', async () => {
      const track = setup()
      track.client.respond = () => ({ trackProperties: [new MsfCompressionTrackProperty(1) as never] })
      await track.reader.start()
      track.client.last.push(object(1, 0, await gzip(text('t')), 9))
      await track.settle()
      expect(track.log).toEqual(['i 1.0 t'])
      await track.reader.stop()

      const per = setup()
      await per.reader.start()
      per.client.last.push(object(1, 0, await gzip(text('o')), 1), object(1, 1, 'x', 7), object(1, 2, 'y'))
      await per.settle()
      expect(per.log).toEqual(['i 1.0 o', 'stale Unsupported MSF compression algorithm 7'])
      await per.reader.stop()

      const unsupported = setup()
      unsupported.client.respond = () => ({
        trackProperties: [KeyValuePair.tryNewVarInt(0x78, 5) as never],
      })
      await unsupported.reader.start()
      expect(unsupported.log).toEqual(['stale Unsupported MSF compression algorithm 5 (MSF 12.1.1)'])
      expect(unsupported.client.sent('unsubscribe')).toEqual([0n])
    })

    test('a rejected subscribe throws; a lost stream resubscribes', async () => {
      const { client, reader } = setup()
      client.respond = () => refuse()
      await expect(reader.start()).rejects.toMatchObject({ code: 'UNSUPPORTED_SOURCE' })
      client.respond = () => ({})
      await reader.start()
      client.last.reset()
      await flush()
      expect(client.sent('subscribe')).toHaveLength(3)
      await reader.stop()
    })
  })
}
