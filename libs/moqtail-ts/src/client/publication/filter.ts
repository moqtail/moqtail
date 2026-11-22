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

import { isVarInt } from '@/model/common/pair'
import type { KeyValuePair } from '@/model/common/pair'
import type { MoqtObject } from '@/model/data/object'
import { InvalidFilterError, RANGE_FILTERS, validRanges } from '@/model/parameters'
import type { MessageParameters, Range } from '@/model/parameters'
import { ImmutablePropertiesObjectProperty } from '@/model/property/object_property'

interface FilterSet {
  subgroup?: Range[]
  objectId?: Range[]
  priority?: Range[]
  properties: Map<bigint, Range[]>
}

/**
 * The Range Filters of a request, combined (moqt-22 §9.2.2.8): filters sharing a SetID
 * must all pass, and an Object is sent when any SetID passes. No filter passes everything.
 */
export class CombinedFilter {
  readonly #sets = new Map<number, FilterSet>()

  constructor(params: MessageParameters) {
    const seen = new Set<string>()

    for (const [key] of RANGE_FILTERS) {
      for (const filter of params[key] ?? []) {
        if (filter.setId === undefined) continue
        if (key === 'trackPropertyFilter') {
          throw new InvalidFilterError('CombinedFilter', 'a track property filter selects tracks, not Objects')
        }

        const id = `${key}/${filter.setId}/${filter.propertyType ?? ''}`
        if (seen.has(id)) throw new InvalidFilterError('CombinedFilter', `${key} repeats SetID ${filter.setId}`)

        seen.add(id)

        const ranges = validRanges(key, filter)

        let set = this.#sets.get(filter.setId)
        if (!set) {
          set = { properties: new Map() }
          this.#sets.set(filter.setId, set)
        }

        if (key === 'subgroupFilter') set.subgroup = ranges
        else if (key === 'objectIdFilter') set.objectId = ranges
        else if (key === 'priorityFilter') set.priority = ranges
        else set.properties.set(filter.propertyType!, ranges)
      }
    }
  }

  get isEmpty(): boolean {
    return this.#sets.size === 0
  }

  /** A datagram has no subgroup, which no subgroup filter passes. */
  matches(obj: MoqtObject, subgroupId: bigint | null = obj.subgroupId): boolean {
    if (this.isEmpty) return true

    for (const set of this.#sets.values()) {
      if (set.subgroup && (subgroupId === null || !within(set.subgroup, subgroupId))) continue
      if (set.objectId && !within(set.objectId, obj.location.object)) continue
      if (set.priority && !within(set.priority, BigInt(obj.publisherPriority))) continue
      if (!propertiesPass(set.properties, obj.properties)) continue

      return true
    }

    return false
  }
}

function within(ranges: readonly Range[], value: bigint): boolean {
  return ranges.some((r) => value >= r.start && (r.end === undefined || value <= r.end))
}

/** A property filter reads varint properties, ImmutableProperties' included. */
function propertiesPass(required: Map<bigint, Range[]>, properties: KeyValuePair[] | null): boolean {
  if (required.size === 0) return true

  const values = new Map<bigint, bigint>()
  const collect = (pairs: readonly KeyValuePair[]) => {
    for (const pair of pairs) {
      if (isVarInt(pair)) {
        if (!values.has(pair.typeValue)) values.set(pair.typeValue, pair.value)
        continue
      }

      const immutable = ImmutablePropertiesObjectProperty.fromKeyValuePair(pair)
      if (immutable) collect(immutable.properties)
    }
  }

  collect(properties ?? [])

  for (const [type, ranges] of required) {
    const value = values.get(type)
    if (value === undefined || !within(ranges, value)) return false
  }

  return true
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { Location } = await import('@/model/common/location')
  const { KeyValuePair } = await import('@/model/common/pair')
  const { ObjectForwardingPreference, ObjectStatus } = await import('@/model/data/constant')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { MoqtObject } = await import('@/model/data/object')
  const { encodeRanges } = await import('@/model/parameters')
  type RangeFilter = import('@/model/parameters').RangeFilter

  const ftn = FullTrackName.tryNew('ns', 'track')
  const object = (group: bigint, id: bigint, priority = 128, properties: KeyValuePair[] | null = null) =>
    MoqtObject.newWithPayload(
      ftn,
      new Location(group, id),
      priority,
      ObjectForwardingPreference.Subgroup,
      0n,
      properties,
      new Uint8Array([1]),
    )

  const filter = (setId: number, ranges: Range[], propertyType?: bigint): RangeFilter =>
    propertyType === undefined
      ? { setId, deltas: encodeRanges(ranges) }
      : { setId, propertyType, deltas: encodeRanges(ranges) }

  describe('CombinedFilter', () => {
    test('no filter passes everything', () => {
      const net = new CombinedFilter({ forward: true, objectIdFilter: [{ deltas: [] }] })

      expect(net.isEmpty).toBe(true)
      expect(net.matches(object(0n, 9n))).toBe(true)
    })

    test('ranges within one filter are ORed', () => {
      const ranges = [
        { start: 0n, end: 1n },
        { start: 5n, end: 5n },
      ]
      const net = new CombinedFilter({ objectIdFilter: [filter(0, ranges)] })

      expect([0n, 1n, 2n, 5n, 6n].map((id) => net.matches(object(0n, id)))).toEqual([true, true, false, true, false])
    })

    test('filters sharing a SetID are ANDed, and SetIDs are ORed', () => {
      const net = new CombinedFilter({
        // Set 0: object 0 at high priority (low value). Set 1: anything in subgroup 3.
        objectIdFilter: [filter(0, [{ start: 0n, end: 0n }])],
        priorityFilter: [filter(0, [{ start: 0n, end: 63n }])],
        subgroupFilter: [filter(1, [{ start: 3n, end: 3n }])],
      })

      expect(net.matches(object(0n, 0n, 10), 0n)).toBe(true)
      expect(net.matches(object(0n, 0n, 200), 0n)).toBe(false)
      expect(net.matches(object(0n, 4n, 200), 3n)).toBe(true)
      expect(net.matches(object(0n, 4n, 10), 0n)).toBe(false)
    })

    test('a subgroup filter passes no datagram', () => {
      const net = new CombinedFilter({ subgroupFilter: [filter(0, [{ start: 0n }])] })

      expect(net.matches(object(0n, 0n), 0n)).toBe(true)
      expect(net.matches(object(0n, 0n), null)).toBe(false)
    })

    test('a property filter needs the property, and looks inside ImmutableProperties', () => {
      const net = new CombinedFilter({ objectPropertyFilter: [filter(0, [{ start: 10n, end: 20n }], 0x3cn)] })
      const prop = (value: bigint) => KeyValuePair.tryNewVarInt(0x3cn, value)

      expect(net.matches(object(0n, 0n, 128, [prop(15n)]))).toBe(true)
      expect(net.matches(object(0n, 0n, 128, [prop(25n)]))).toBe(false)
      expect(net.matches(object(0n, 0n, 128, null))).toBe(false)

      const immutable = new ImmutablePropertiesObjectProperty([prop(12n)]).toKeyValuePair()

      expect(net.matches(object(0n, 0n, 128, [immutable]))).toBe(true)
    })

    test('a repeated Type, SetID and Property Type is invalid', () => {
      const repeated = [filter(0, [{ start: 0n, end: 0n }]), filter(0, [{ start: 4n, end: 4n }])]

      expect(() => new CombinedFilter({ objectIdFilter: repeated })).toThrow(InvalidFilterError)

      // A different Property Type under the same SetID is a separate filter
      const distinct = [filter(0, [{ start: 0n }], 0x3cn), filter(0, [{ start: 0n }], 0x3en)]

      expect(() => new CombinedFilter({ objectPropertyFilter: distinct })).not.toThrow()
    })

    test('a filter that cannot be applied is invalid', () => {
      const priority = { priorityFilter: [filter(0, [{ start: 0n, end: 300n }])] }

      expect(() => new CombinedFilter(priority)).toThrow(InvalidFilterError)

      const track = { trackPropertyFilter: [filter(0, [{ start: 0n }], 0x3cn)] }

      expect(() => new CombinedFilter(track)).toThrow(InvalidFilterError)
    })

    test('a filter set with no ranges passes nothing', () => {
      const net = new CombinedFilter({ objectIdFilter: [filter(0, [])] })

      expect(net.matches(object(0n, 0n))).toBe(false)
    })

    test('an end-of-group status object is filtered like any other', () => {
      const net = new CombinedFilter({ objectIdFilter: [filter(0, [{ start: 0n, end: 0n }])] })
      const end = MoqtObject.newWithStatus(
        ftn,
        new Location(0n, 3n),
        128,
        ObjectForwardingPreference.Subgroup,
        0n,
        null,
        ObjectStatus.EndOfGroup,
      )

      expect(net.matches(end)).toBe(false)
    })
  })
}
