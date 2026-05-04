/**
 * Copyright 2025 The MOQtail Authors
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

import { ObjectCache } from './object_cache'
import { Location } from '../../model/common/location'
import { MoqtObject } from '../../model/data/object'
import { logger } from '../../util/logger'

// TODO: Consider switching to readable stream
/**
 * Source of already-produced (historical) objects for a track.
 * Backed by an {@link ObjectCache} allowing range retrieval using protocol {@link Location}s.
 * Implementations MUST return objects in ascending location order and MAY return an empty array
 * if the requested window is outside the cached range.
 */
export interface PastObjectSource {
  /** Underlying cache from which objects are served */
  readonly cache: ObjectCache
  /**
   * Fetch a (closed) range of objects. `start`/`end` are inclusive when provided.
   * Omitted bounds mean: from earliest cached (when `start` undefined) or up to latest cached (when `end` undefined).
   */
  getRange(start?: Location, end?: Location): Promise<MoqtObject[]>
}

/**
 * Push-oriented live object feed. Wraps a {@link https://developer.mozilla.org/docs/Web/API/ReadableStream | ReadableStream} plus lightweight event subscription helpers.
 * Implementations advance {@link LiveObjectSource.largestLocation | largestLocation} monotonically as objects arrive.
 */
export interface LiveObjectSource {
  /** Continuous stream yielding objects as they are produced */
  readonly stream: ReadableStream<MoqtObject>
  /** Highest (latest) location observed so far; undefined until first object */
  readonly largestLocation: Location | undefined
  /** Register a listener invoked (async) for each new object. Returns an unsubscribe function. */
  onNewObject(listener: (obj: MoqtObject) => void): () => void
  /** Register a listener invoked when the live stream ends (normal or error). Returns an unsubscribe function. */
  onDone(listener: () => void): () => void
  /** Stop ingestion and release underlying reader (idempotent). */
  stop(): void
}

/**
 * Aggregates optional historical (`past`) and (`live`) sources for a single track.
 * Either facet may be omitted:
 * - VOD / static content: supply only {@link TrackSource.past | past}
 * - Pure live: supply only {@link TrackSource.live | live}
 * - Hybrid (catch-up + live tail): supply both.
 *
 * Priority handling note: publisher priority is defined on the Track metadata (see `Track.publisherPriority`).
 * The library rounds non-integer values and clamps priority into [0,255] there; this interface simply
 * expresses what content is available, independent of priority semantics.
 */
export interface TrackSource {
  /** Historical object access (optional) */
  readonly past?: PastObjectSource
  /** Live object feed (optional) */
  readonly live?: LiveObjectSource
}

export class StaticTrackSource implements PastObjectSource {
  readonly cache: ObjectCache

  constructor(cache: ObjectCache) {
    this.cache = cache
  }

  async getRange(start?: Location, end?: Location): Promise<MoqtObject[]> {
    return this.cache.getRange(start, end)
  }
}

export class LiveTrackSource implements LiveObjectSource {
  readonly stream: ReadableStream<MoqtObject>
  readonly #listeners = new Set<(obj: MoqtObject) => void>()
  readonly #doneListeners = new Set<() => void>()

  #largestLocation: Location | undefined
  #ingestActive = false
  #reader?: ReadableStreamDefaultReader<MoqtObject>

  constructor(stream: ReadableStream<MoqtObject>) {
    this.stream = stream
    this.#startIngest()
  }

  get largestLocation(): Location | undefined {
    return this.#largestLocation
  }

  async #startIngest() {
    if (this.#ingestActive) return
    this.#ingestActive = true

    try {
      this.#reader = this.stream.getReader()

      while (this.#ingestActive) {
        const { value, done } = await this.#reader.read()
        if (done) break

        this.#largestLocation = value.location

        for (const listener of this.#listeners) {
          listener(value)
        }
      }
    } catch (error) {
      logger.error('track/content_source', 'Error during live object ingestion:', error)
    } finally {
      this.#ingestActive = false
      this.#reader?.releaseLock()

      for (const doneListener of this.#doneListeners) {
        Promise.resolve().then(() => doneListener())
      }
    }
  }

  onNewObject(listener: (obj: MoqtObject) => Promise<void> | void): () => void {
    let queue = Promise.resolve()

    // Per-listener promise queue: dispatch stays fire-and-forget but each listener's callbacks
    // are chained so obj2 only starts after obj1 resolves.
    const queuedListener = (obj: MoqtObject) => {
      queue = queue
        .then(() => listener(obj))
        .catch((err) => {
          logger.error('track/content_source', 'Error in subscriber listener:', err)
        })
    }

    this.#listeners.add(queuedListener)

    return () => this.#listeners.delete(queuedListener)
  }

  onDone(listener: () => void): () => void {
    this.#doneListeners.add(listener)
    return () => this.#doneListeners.delete(listener)
  }

  stop(): void {
    this.#ingestActive = false
    this.#reader?.cancel()
  }
}

export class HybridTrackSource implements TrackSource {
  readonly past: PastObjectSource
  readonly live: LiveObjectSource

  constructor(cache: ObjectCache, stream: ReadableStream<MoqtObject>) {
    this.past = new StaticTrackSource(cache)
    this.live = new LiveTrackSource(stream)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('LiveTrackSource', () => {
    test('queue preserves strict FIFO order despite async delays', async () => {
      const writtenObjects: bigint[] = []
      let streamCount = 0

      const mockSendStream = {
        write: async (obj: any) => {
          writtenObjects.push(obj.objectId)
        },
      }

      const mockWebTransport = {
        createUnidirectionalStream: async () => {
          streamCount++
          if (streamCount === 1) {
            await new Promise((resolve) => setTimeout(resolve, 50))
          } else {
            await Promise.resolve()
          }
          return {}
        },
      }

      let streamController!: ReadableStreamDefaultController<any>
      const stream = new ReadableStream({
        start(controller) {
          streamController = controller
        },
      })

      const trackSource = new LiveTrackSource(stream)

      trackSource.onNewObject(async (obj) => {
        await mockWebTransport.createUnidirectionalStream()
        await mockSendStream.write(obj)
      })

      await new Promise((resolve) => setTimeout(resolve, 0))

      streamController.enqueue({ objectId: 0n, location: { group: 5n, object: 0n } } as any)
      streamController.enqueue({ objectId: 1n, location: { group: 5n, object: 1n } } as any)

      await new Promise((resolve) => setTimeout(resolve, 100))

      expect(writtenObjects).toEqual([0n, 1n])

      trackSource.stop()
    })

    test('queue recovers gracefully if a listener throws an error', async () => {
      const writtenObjects: bigint[] = []

      let streamController!: ReadableStreamDefaultController<any>
      const stream = new ReadableStream({
        start(controller) {
          streamController = controller
        },
      })

      const trackSource = new LiveTrackSource(stream)

      trackSource.onNewObject(async (obj) => {
        if (obj.objectId === 0n) {
          throw new Error('Simulated WebTransport connection blip!')
        }
        writtenObjects.push(obj.objectId)
      })

      await new Promise((resolve) => setTimeout(resolve, 0))

      streamController.enqueue({ objectId: 0n, location: { group: 5n, object: 0n } } as any)
      streamController.enqueue({ objectId: 1n, location: { group: 5n, object: 1n } } as any)

      await new Promise((resolve) => setTimeout(resolve, 100))
      expect(writtenObjects).toEqual([1n])

      trackSource.stop()
    })
  })
}
