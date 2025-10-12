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

import { RequestIdError } from '../error'
import { FullTrackName } from './full_track_name'

/**
 * A bidirectional map between requestId (bigint) and full track names.
 * Used to efficiently look up either by requestId or by name, enforcing uniqueness.
 *
 * @public
 */
export class RequestIdMap {
  private requestIdToName = new Map<bigint, FullTrackName>()
  private nameToRequestId = new Map<FullTrackName, bigint>()

  /**
   * @public
   * Adds a mapping between a requestId and a full track name.
   *
   * @param requestId - The requestId to associate.
   * @param name - The full track name to associate.
   * @throws :{@link RequestIdError} if the requestId or name is already mapped to a different value.
   */
  addMapping(requestId: bigint, name: FullTrackName): void {
    if (this.requestIdToName.has(requestId)) {
      const existingName = this.requestIdToName.get(requestId)
      if (existingName === name) return
      throw new RequestIdError(
        'RequestIdMap::addMapping(existingName)',
        `Full track name already exists for requestId: ${requestId}`,
      )
    }
    if (this.nameToRequestId.has(name)) {
      const existingRequestId = this.nameToRequestId.get(name)
      if (existingRequestId === requestId) return
      throw new RequestIdError(
        'RequestIdMap::addMapping(existingRequestId)',
        `A requestId already exists for full track name: ${name}`,
      )
    }
    this.requestIdToName.set(requestId, name)
    this.nameToRequestId.set(name, requestId)
  }

  /**
   * @public
   * Gets the full track name for a given requestId.
   *
   * @param requestId - The requestId to look up.
   * @returns The associated full track name.
   * @throws :{@link RequestIdError} if the requestId does not exist.
   */
  getNameByRequestId(requestId: bigint): FullTrackName {
    const name = this.requestIdToName.get(requestId)
    if (!name)
      throw new RequestIdError('RequestIdMap::getNameByRequestId(name)', `RequestId: ${requestId} doesn't exist`)
    return name
  }

  /**
   * @public
   * Gets the requestId for a given full track name.
   *
   * @param name - The full track name to look up.
   * @returns The associated requestId.
   * @throws :{@link RequestIdError} if the name does not exist.
   */
  getRequestIdByName(name: FullTrackName): bigint {
    const requestId = this.nameToRequestId.get(name)
    if (requestId === undefined)
      throw new RequestIdError('RequestIdMap::getRequestIdByName(requestId)', `Name does not exist`)
    return requestId
  }

  /**
   * @public
   * Removes a mapping by requestId.
   *
   * @param requestId - The requestId to remove.
   * @returns The removed full track name, or undefined if not found.
   */
  removeMappingByRequestId(requestId: bigint): FullTrackName | undefined {
    const name = this.requestIdToName.get(requestId)
    if (name) {
      this.requestIdToName.delete(requestId)
      this.nameToRequestId.delete(name)
      return name
    }
    return undefined
  }

  /**
   * @public
   * Removes a mapping by full track name.
   *
   * @param name - The full track name to remove.
   * @returns The removed requestId, or undefined if not found.
   */
  removeMappingByName(name: FullTrackName): bigint | undefined {
    const requestId = this.nameToRequestId.get(name)
    if (requestId !== undefined) {
      this.nameToRequestId.delete(name)
      this.requestIdToName.delete(requestId)
      return requestId
    }
    return undefined
  }

  /**
   * @public
   * Checks if the map contains a given requestId.
   *
   * @param requestId - The requestId to check.
   * @returns True if the requestId exists, false otherwise.
   */
  containsRequestId(requestId: bigint): boolean {
    return this.requestIdToName.has(requestId)
  }

  /**
   * @public
   * Checks if the map contains a given full track name.
   *
   * @param name - The full track name to check.
   * @returns True if the name exists, false otherwise.
   */
  containsName(name: FullTrackName): boolean {
    return this.nameToRequestId.has(name)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('RequestIdMap', () => {
    test('add and get mapping roundtrip', () => {
      const map = new RequestIdMap()
      const requestId = 42n
      const name = FullTrackName.tryNew('namespace/test', 'bamboozeled')
      map.addMapping(requestId, name)
      expect(map.getNameByRequestId(requestId)).toEqual(name)
      expect(map.getRequestIdByName(name)).toBe(requestId)
    })
    test('add duplicate requestId error', () => {
      const map = new RequestIdMap()
      const requestId = 1n
      const name1 = FullTrackName.tryNew('namespace/test', 'bamboozeled')
      const name2 = FullTrackName.tryNew('namespace/test/yeeahboii', 'bamboozeled')
      map.addMapping(requestId, name1)
      expect(() => map.addMapping(requestId, name2)).toThrow()
    })
  })
}
