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

import { KeyValuePair, isVarInt } from '../../common/pair'
import { MoqtObject } from '../../data/object'
import { ImmutablePropertiesObjectProperty } from '../../property/object_property'
import type { MessageParameter } from '../message_parameter'
import { InvalidFilterError, Range, RangeFilter, RangeFilterKind } from './range_filter'

/** The criteria one SetID holds. Each one present must pass; an absent one passes anything. */
interface FilterSet {
  subgroup?: Range[]
  objectId?: Range[]
  priority?: Range[]
  /** Property Type to the Ranges its value must fall in. */
  properties: Map<bigint, Range[]>
}

const within = (ranges: readonly Range[], value: bigint): boolean =>
  ranges.some((r) => value >= r.start && (r.end === undefined || value <= r.end))

/**
 * The object filter a request's Range Filters add up to.
 *
 * Ranges within one filter are OR'd, filters sharing a SetID are AND'd, and the
 * SetIDs are OR'd with each other. No filter at all passes every Object.
 */
export class CombinedFilter {
  readonly #sets = new Map<number, FilterSet>()

  /** How many Ranges the filters hold, which is what a range limit counts. */
  readonly rangeCount: number

  /**
   * Builds the filter from every Range Filter in `params`; anything else, and a
   * zero-length removal, is ignored.
   *
   * @throws :{@link InvalidFilterError} For a filter that cannot be applied: one
   * that repeats a Type, SetID and Property Type already given, one with an odd
   * Property Type, a priority above 255 or a bound past 2^64-1, or a track filter,
   * which selects tracks rather than Objects.
   */
  constructor(params: readonly MessageParameter[]) {
    let rangeCount = 0
    const seen = new Set<string>()
    for (const param of params) {
      if (!(param instanceof RangeFilter) || param.isRemoval) continue
      const filter = param
      if (filter.kind === RangeFilterKind.TrackProperty) {
        throw new InvalidFilterError('CombinedFilter', 'a track property filter selects tracks, not Objects')
      }
      const key = `${filter.kind}/${filter.setId}/${filter.propertyType ?? ''}`
      if (seen.has(key)) {
        throw new InvalidFilterError(
          'CombinedFilter',
          `${RangeFilterKind[filter.kind]} filter repeats SetID ${filter.setId}` +
            (filter.propertyType !== undefined ? ` for property 0x${filter.propertyType.toString(16)}` : ''),
        )
      }
      seen.add(key)

      const ranges = filter.validate()
      rangeCount += ranges.length
      let set = this.#sets.get(filter.setId!)
      if (!set) {
        set = { properties: new Map() }
        this.#sets.set(filter.setId!, set)
      }
      switch (filter.kind) {
        case RangeFilterKind.Subgroup:
          set.subgroup = ranges
          break
        case RangeFilterKind.ObjectId:
          set.objectId = ranges
          break
        case RangeFilterKind.Priority:
          set.priority = ranges
          break
        case RangeFilterKind.ObjectProperty:
          set.properties.set(filter.propertyType!, ranges)
          break
      }
    }
    this.rangeCount = rangeCount
  }

  /** Whether the filter passes everything. */
  get isEmpty(): boolean {
    return this.#sets.size === 0
  }

  /**
   * Whether `obj` passes.
   *
   * @param subgroupId - The Subgroup ID `obj` travels in. It is passed in because an
   * Object whose Subgroup ID is its subgroup's first Object ID does not know it
   * itself. `null` — a datagram — has no subgroup, so no subgroup filter passes it.
   */
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

/**
 * Whether each required Property is present with a value in its Ranges. One the
 * Object does not carry fails. Properties inside ImmutableProperties count as the
 * Object's own.
 */
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
  const { encodeRanges } = await import('./range_filter')
  const { Location } = await import('../../common/location')
  const { FullTrackName } = await import('../../data/full_track_name')
  const { ObjectForwardingPreference, ObjectStatus } = await import('../../data/constant')
  const { Forward } = await import('./forward')

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
  const filter = (kind: RangeFilterKind, setId: number, ranges: Range[], propertyType?: bigint) =>
    new RangeFilter(kind, setId, propertyType, encodeRanges(ranges))

  describe('CombinedFilter', () => {
    test('no filter passes everything', () => {
      const net = new CombinedFilter([new Forward(true), new RangeFilter(RangeFilterKind.ObjectId)])
      expect(net.isEmpty).toBe(true)
      expect(net.matches(object(0n, 9n))).toBe(true)
    })

    test('ranges within one filter are ORed', () => {
      const net = new CombinedFilter([
        filter(RangeFilterKind.ObjectId, 0, [
          { start: 0n, end: 1n },
          { start: 5n, end: 5n },
        ]),
      ])
      expect([0n, 1n, 2n, 5n, 6n].map((id) => net.matches(object(0n, id)))).toEqual([true, true, false, true, false])
      expect(net.rangeCount).toBe(2)
    })

    test('filters sharing a SetID are ANDed, and SetIDs are ORed', () => {
      const net = new CombinedFilter([
        // Set 0: object 0 at high priority (low value).
        filter(RangeFilterKind.ObjectId, 0, [{ start: 0n, end: 0n }]),
        filter(RangeFilterKind.Priority, 0, [{ start: 0n, end: 63n }]),
        // Set 1: anything in subgroup 3.
        filter(RangeFilterKind.Subgroup, 1, [{ start: 3n, end: 3n }]),
      ])
      expect(net.matches(object(0n, 0n, 10), 0n)).toBe(true)
      expect(net.matches(object(0n, 0n, 200), 0n)).toBe(false)
      expect(net.matches(object(0n, 4n, 200), 3n)).toBe(true)
      expect(net.matches(object(0n, 4n, 10), 0n)).toBe(false)
    })

    test('a subgroup filter passes no datagram', () => {
      const net = new CombinedFilter([filter(RangeFilterKind.Subgroup, 0, [{ start: 0n }])])
      expect(net.matches(object(0n, 0n), 0n)).toBe(true)
      expect(net.matches(object(0n, 0n), null)).toBe(false)
    })

    test('a property filter needs the property, and looks inside ImmutableProperties', () => {
      const net = new CombinedFilter([filter(RangeFilterKind.ObjectProperty, 0, [{ start: 10n, end: 20n }], 0x3cn)])
      const prop = (value: bigint) => KeyValuePair.tryNewVarInt(0x3cn, value)
      expect(net.matches(object(0n, 0n, 128, [prop(15n)]))).toBe(true)
      expect(net.matches(object(0n, 0n, 128, [prop(25n)]))).toBe(false)
      expect(net.matches(object(0n, 0n, 128, null))).toBe(false)
      const immutable = new ImmutablePropertiesObjectProperty([prop(12n)]).toKeyValuePair()
      expect(net.matches(object(0n, 0n, 128, [immutable]))).toBe(true)
    })

    test('a repeated Type, SetID and Property Type is invalid', () => {
      expect(
        () =>
          new CombinedFilter([
            filter(RangeFilterKind.ObjectId, 0, [{ start: 0n, end: 0n }]),
            filter(RangeFilterKind.ObjectId, 0, [{ start: 4n, end: 4n }]),
          ]),
      ).toThrow(InvalidFilterError)
      // A different Property Type under the same SetID is a separate filter.
      expect(
        () =>
          new CombinedFilter([
            filter(RangeFilterKind.ObjectProperty, 0, [{ start: 0n }], 0x3cn),
            filter(RangeFilterKind.ObjectProperty, 0, [{ start: 0n }], 0x3en),
          ]),
      ).not.toThrow()
    })

    test('a filter that cannot be applied is invalid', () => {
      expect(() => new CombinedFilter([filter(RangeFilterKind.Priority, 0, [{ start: 0n, end: 300n }])])).toThrow(
        InvalidFilterError,
      )
      expect(() => new CombinedFilter([filter(RangeFilterKind.TrackProperty, 0, [{ start: 0n }], 0x3cn)])).toThrow(
        InvalidFilterError,
      )
    })

    test('a filter set with no ranges passes nothing', () => {
      const net = new CombinedFilter([filter(RangeFilterKind.ObjectId, 0, [])])
      expect(net.matches(object(0n, 0n))).toBe(false)
    })

    test('an end-of-group status object is filtered like any other', () => {
      const net = new CombinedFilter([filter(RangeFilterKind.ObjectId, 0, [{ start: 0n, end: 0n }])])
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
