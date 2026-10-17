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

import { Location } from '../../model/common/location'
import { MoqtObject } from '../../model/data/object'

/**
 * In‑memory index of {@link MoqtObject} instances keyed by their {@link Location} (group/subgroup/object),
 * providing ordered insertion and range queries.
 *
 * Implementations MUST keep objects sorted ascending by (groupId, objectId) so binary searches can
 * locate insertion points / range bounds efficiently. Subgroup ordinality is currently ignored for ordering
 * (adjust if protocol semantics require finer granularity later).
 *
 * @example Typical usage
 * ```ts
 * const cache: ObjectCache = new MemoryObjectCache()
 * cache.add(obj)
 * const window = cache.getRange(startLoc, endLoc)
 * const exact = cache.getByLocation(loc)
 * ```
 *
 * Concurrency: Implementations here are not thread‑safe; callers avoid concurrent mutation from workers.
 */
export interface ObjectCache {
  /** Insert a new {@link MoqtObject}, preserving sorted order (duplicates allowed or replaced impl‑defined). */
  add(obj: MoqtObject): void
  /** Return a shallow copy array of objects whose Location is \>= start and \< end (end exclusive). */
  getRange(start?: Location, end?: Location): MoqtObject[]
  /** Return the object whose Location exactly matches (group & object) or undefined if absent. */
  getByLocation(location: Location): MoqtObject | undefined
  /** Current number of cached objects. */
  size(): number
  /** Remove all cached objects. */
  clear(): void
}

export class MemoryObjectCache implements ObjectCache {
  private objects: MoqtObject[] = []

  // Insert and keep sorted by Location
  add(obj: MoqtObject): void {
    const idx = this._findInsertIndex(obj)
    this.objects.splice(idx, 0, obj)
  }

  getRange(start?: Location, end?: Location): MoqtObject[] {
    const startIdx = start ? this._findIndex(start, false) : 0
    const endIdx = end ? this._findIndex(end, true) : this.objects.length
    return this.objects.slice(startIdx, endIdx)
  }

  getByLocation(location: Location): MoqtObject | undefined {
    const idx = this._findIndex(location, false)
    const obj = this.objects[idx]
    if (obj && obj.groupId === location.group && obj.objectId === location.object) {
      return obj
    }
    return undefined
  }

  size(): number {
    return this.objects.length
  }

  clear(): void {
    this.objects = []
  }

  // Binary search helpers
  private _findInsertIndex(obj: MoqtObject): number {
    let low = 0,
      high = this.objects.length
    while (low < high) {
      const mid = (low + high) >> 1
      const midObj = this.objects[mid]
      if (!midObj) break
      const cmp = new Location(midObj.groupId, midObj.objectId).compare(new Location(obj.groupId, obj.objectId))
      if (cmp < 0) low = mid + 1
      else high = mid
    }
    return low
  }

  private _findIndex(location: Location, exclusive: boolean): number {
    let low = 0,
      high = this.objects.length
    while (low < high) {
      const mid = (low + high) >> 1
      const midObj = this.objects[mid]
      if (!midObj) break
      const cmp = new Location(midObj.groupId, midObj.objectId).compare(location)
      if (cmp < 0 || (exclusive && cmp === 0)) low = mid + 1
      else high = mid
    }
    return low
  }
}

export class RingBufferObjectCache implements ObjectCache {
  private buffer: MoqtObject[]
  private maxSize: number

  constructor(maxSize: number = 100) {
    this.buffer = []
    this.maxSize = maxSize
  }

  add(obj: MoqtObject): void {
    // Insert in sorted order (by Location)
    const idx = this._findInsertIndex(obj)
    this.buffer.splice(idx, 0, obj)
    // Evict oldest if over capacity
    if (this.buffer.length > this.maxSize) {
      this.buffer.shift()
    }
  }

  getRange(start?: Location, end?: Location): MoqtObject[] {
    const startIdx = start ? this._findIndex(start, false) : 0
    const endIdx = end ? this._findIndex(end, true) : this.buffer.length
    return this.buffer.slice(startIdx, endIdx)
  }

  getByLocation(location: Location): MoqtObject | undefined {
    const idx = this._findIndex(location, false)
    const obj = this.buffer[idx]
    if (obj && obj.groupId === location.group && obj.objectId === location.object) {
      return obj
    }
    return undefined
  }

  size(): number {
    return this.buffer.length
  }

  clear(): void {
    this.buffer = []
  }

  // Binary search helpers
  private _findInsertIndex(obj: MoqtObject): number {
    let low = 0,
      high = this.buffer.length
    while (low < high) {
      const mid = (low + high) >> 1
      const midObj = this.buffer[mid]
      if (!midObj) break
      const cmp = new Location(midObj.groupId, midObj.objectId).compare(new Location(obj.groupId, obj.objectId))
      if (cmp < 0) low = mid + 1
      else high = mid
    }
    return low
  }

  private _findIndex(location: Location, exclusive: boolean): number {
    let low = 0,
      high = this.buffer.length
    while (low < high) {
      const mid = (low + high) >> 1
      const midObj = this.buffer[mid]
      if (!midObj) break
      const cmp = new Location(midObj.groupId, midObj.objectId).compare(location)
      if (cmp < 0 || (exclusive && cmp === 0)) low = mid + 1
      else high = mid
    }
    return low
  }
}
