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
import { createLogger } from '../../util/logger'

const logger = createLogger('track/content_source')

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
          Promise.resolve().then(() => listener(value))
        }
      }
    } catch (error) {
      logger.error('Error during live object ingestion:', error)
    } finally {
      this.#ingestActive = false
      this.#reader?.releaseLock()

      for (const doneListener of this.#doneListeners) {
        Promise.resolve().then(() => doneListener())
      }
    }
  }

  onNewObject(listener: (obj: MoqtObject) => void): () => void {
    this.#listeners.add(listener)
    return () => this.#listeners.delete(listener)
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
