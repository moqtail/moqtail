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

import { SubgroupHeaderType } from '@/model'
import { SubgroupHeader } from '@/model/data/subgroup_header'
import { MoqtObject } from '@/model/data/object'

/** Groups kept behind the newest in {@link SubgroupIds}, for objects that arrive late. */
const GROUPS_KEPT = 4n

/**
 * Resolves the Subgroup ID of Objects that leave it to their subgroup's first Object
 * ID (a `null` {@link MoqtObject.subgroupId}). An Object cannot tell that on its own
 * once the first one has gone by, so the first Object ID of each group is noted as
 * the Objects arrive, whether or not they are then sent.
 *
 * This assumes one subgroup per group, which is how this client publishes.
 */
export class SubgroupIds {
  readonly #firstObject = new Map<bigint, bigint>()

  /** Notes `obj`. Call it for every Object the track produces, sent or not. */
  note(obj: MoqtObject): void {
    const group = obj.location.group
    const first = this.#firstObject.get(group)
    if (first !== undefined && first <= obj.location.object) return
    this.#firstObject.set(group, obj.location.object)
    for (const old of this.#firstObject.keys()) {
      if (old + GROUPS_KEPT < group) this.#firstObject.delete(old)
    }
  }

  /**
   * The Subgroup ID `obj` travels in, or `null` for a datagram. An Object whose
   * group was never noted is taken to be its subgroup's first.
   */
  of(obj: MoqtObject): bigint | null {
    if (obj.isDatagram()) return null
    return obj.subgroupId ?? this.#firstObject.get(obj.location.group) ?? obj.location.object
  }
}

/**
 * The header for a subgroup stream that starts with `obj`.
 *
 * An Object that leaves its Subgroup ID to the first Object ID only gets that header
 * when it is the first: a stream that starts later — a filter dropped the Objects
 * before it, or the subscription joined mid-group — states the ID, or a receiver
 * would read the wrong one off it.
 */
export function subgroupHeaderFor(
  obj: MoqtObject,
  subgroupId: bigint,
  trackAlias: bigint,
  publisherPriority: number,
  firstObject: boolean,
): SubgroupHeader {
  let headerType = obj.getSubgroupHeaderType(true, false, firstObject)
  if (obj.subgroupId === null && subgroupId !== obj.location.object) {
    const hasProperties = !!obj.properties && obj.properties.length > 0
    headerType = SubgroupHeaderType.fromProperties(hasProperties, 2, true, false, firstObject)
  }
  const explicit = SubgroupHeaderType.hasExplicitSubgroupId(headerType) ? subgroupId : undefined
  return new SubgroupHeader(headerType, trackAlias, obj.location.group, explicit, publisherPriority)
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { Location } = await import('@/model/common/location')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { ObjectForwardingPreference } = await import('@/model/data/constant')

  const ftn = FullTrackName.tryNew('ns', 'track')
  const object = (group: bigint, id: bigint, subgroupId: bigint | null = null) =>
    MoqtObject.newWithPayload(
      ftn,
      new Location(group, id),
      128,
      ObjectForwardingPreference.Subgroup,
      subgroupId,
      null,
      new Uint8Array([1]),
    )

  describe('SubgroupIds', () => {
    test('resolves a first-object-ID subgroup from the first object noted', () => {
      const ids = new SubgroupIds()
      ids.note(object(3n, 2n))
      ids.note(object(3n, 5n))
      expect(ids.of(object(3n, 5n))).toBe(2n)
      // An explicit Subgroup ID is the Object's own.
      expect(ids.of(object(3n, 5n, 7n))).toBe(7n)
      // A group never noted starts at the Object itself.
      expect(ids.of(object(4n, 1n))).toBe(1n)
    })
  })

  describe('subgroupHeaderFor', () => {
    test('leaves the ID to the first object when the stream starts there', () => {
      const header = subgroupHeaderFor(object(3n, 2n), 2n, 1n, 128, true)
      expect(SubgroupHeaderType.hasExplicitSubgroupId(header.type)).toBe(false)
    })

    test('states the ID when the stream starts after the first object', () => {
      const header = subgroupHeaderFor(object(3n, 5n), 2n, 1n, 128, false)
      expect(SubgroupHeaderType.hasExplicitSubgroupId(header.type)).toBe(true)
      expect(header.subgroupId).toBe(2n)
    })
  })
}
