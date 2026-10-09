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

import { ByteBuffer, Location } from '../../common'
import { KeyValuePair } from '../../common/pair'
import { ProtocolViolationError } from '../../error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'

/** 2^64 - 1, the clamp and overflow boundary §5.1.2 states for group arithmetic. */
const MAX_U64 = (1n << 64n) - 1n

/**
 * An inclusive range of Locations, from §5.1.2.
 *
 * ```text
 * LOCATION_FILTER Parameter {
 *   Parameter Type (vi64) = 0x21,
 *   Length (vi64),
 *   [StartGroup (vi64),]
 *   [StartObject (vi64),]
 *   [EndGroupDelta (vi64),]
 *   [EndObject (vi64),]
 * }
 * ```
 *
 * There is no Filter Type on the wire: the four fields are a prefix, and how many of
 * them are present is what selects the meaning. That is why a field cannot be supplied
 * without its predecessors, and why the no-field form is a real value — a zero-length
 * filter removes the filter in REQUEST_UPDATE rather than meaning "unset".
 *
 * One field is relative to the Largest Object; two or more are absolute. {@link resolve}
 * is the only place that distinction is interpreted, so a caller never has to count
 * fields itself.
 */
export class LocationFilter implements Parameter {
  static readonly TYPE = MessageParameterType.LocationFilter

  readonly startGroup: bigint | undefined
  readonly startObject: bigint | undefined
  readonly endGroupDelta: bigint | undefined
  readonly endObject: bigint | undefined

  constructor(startGroup?: bigint, startObject?: bigint, endGroupDelta?: bigint, endObject?: bigint) {
    const fields = [startGroup, startObject, endGroupDelta, endObject]
    const present = fields.findLastIndex((f) => f !== undefined) + 1
    for (let i = 0; i < present; i++) {
      if (fields[i] === undefined) {
        throw new ProtocolViolationError(
          'LocationFilter',
          `field ${i} is absent but field ${present - 1} is present; the fields are a prefix`,
        )
      }
    }
    this.startGroup = startGroup
    this.startObject = startObject
    this.endGroupDelta = endGroupDelta
    this.endObject = endObject
  }

  /** How many of the four optional fields are on the wire; 0..4. */
  get fieldCount(): number {
    return [this.startGroup, this.startObject, this.endGroupDelta, this.endObject].filter((f) => f !== undefined).length
  }

  /** A zero-length filter: no filter at all, and the way REQUEST_UPDATE removes one. */
  get isEmpty(): boolean {
    return this.fieldCount === 0
  }

  /**
   * Whether the start is stated relative to the Largest Object, and so is not knowable
   * without the publisher's view: the one-field form, and the two-zeroed-field Next
   * Object form. With an end bound present the same zeroes are the absolute origin.
   */
  get isRelativeStart(): boolean {
    if (this.fieldCount === 1) return true
    return this.fieldCount === 2 && this.startGroup === 0n && this.startObject === 0n
  }

  /**
   * The start Location when the filter states one outright, and undefined when it is
   * relative or absent. For a subscriber recording what it asked for; a publisher
   * deciding what to send wants {@link resolve}, which takes the Largest Object.
   */
  get statedStart(): Location | undefined {
    if (this.isEmpty || this.isRelativeStart) return undefined
    return new Location(this.startGroup!, this.startObject!)
  }

  /** The absolute end group, when the filter bounds one. */
  get statedEndGroup(): bigint | undefined {
    return this.endGroupDelta === undefined ? undefined : this.startGroup! + this.endGroupDelta
  }

  /** Start at the group after the one holding the Largest Object. */
  static nextGroup(): LocationFilter {
    return new LocationFilter(0n)
  }

  /** Start `groupsBack` groups before the current group; 0 is the current group. */
  static relativeGroups(groupsBack: bigint): LocationFilter {
    return new LocationFilter(groupsBack + 1n)
  }

  /** Start at the object after the Largest Object, or at {0, 0} on an empty track. */
  static nextObject(): LocationFilter {
    return new LocationFilter(0n, 0n)
  }

  /** Start at an absolute Location, open-ended. */
  static absoluteStart(start: Location): LocationFilter {
    return new LocationFilter(start.group, start.object)
  }

  /**
   * An absolute inclusive range. `endGroup` is absolute here and is delta-encoded
   * against the start group on the wire; omitting `endObject` includes the whole end
   * group.
   */
  static absoluteRange(start: Location, endGroup: bigint, endObject?: bigint): LocationFilter {
    if (endGroup < start.group) {
      throw new ProtocolViolationError('LocationFilter.absoluteRange', `end group ${endGroup} precedes ${start.group}`)
    }
    return new LocationFilter(start.group, start.object, endGroup - start.group, endObject)
  }

  toKeyValuePair(): KeyValuePair {
    const buf = new ByteBuffer()
    for (const field of [this.startGroup, this.startObject, this.endGroupDelta, this.endObject]) {
      if (field === undefined) break
      buf.putVI(field)
    }
    return KeyValuePair.tryNewBytes(LocationFilter.TYPE, buf.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): LocationFilter | undefined {
    if (Number(pair.typeValue) !== LocationFilter.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    // Length, not a count, decides how many fields are present, so read to exhaustion
    // rather than to a stated arity. A fifth field is a filter this version cannot read.
    const fields: bigint[] = []
    while (buf.remaining > 0) {
      if (fields.length === 4) {
        throw new ProtocolViolationError('LocationFilter', 'more than four fields in a LOCATION_FILTER')
      }
      fields.push(buf.getVI())
    }
    const filter = new LocationFilter(fields[0], fields[1], fields[2], fields[3])
    // §5.1.2 makes this one a session-closing offence, so it is refused on receipt
    // rather than at the point some consumer happens to add the two together.
    if (filter.endGroupDelta !== undefined && filter.startGroup! + filter.endGroupDelta > MAX_U64) {
      throw new ProtocolViolationError(
        'LocationFilter',
        `StartGroup + EndGroupDelta exceeds 2^64 - 1 (${filter.startGroup} + ${filter.endGroupDelta})`,
      )
    }
    return filter
  }

  /**
   * The absolute range this filter denotes, given the publisher's Largest Object.
   *
   * `largest` is undefined when no content has been delivered yet, which §5.1.2 names
   * for the Next Object form and which the relative form reaches through the same
   * "next group is 0" reading plus its clamp.
   *
   * An absent `endGroup` is open-ended on a subscription; on a Fetch the caller
   * substitutes Largest Object. An absent `endObject` includes the whole end group.
   */
  resolve(largest?: Location): { start: Location; endGroup?: bigint | undefined; endObject?: bigint | undefined } {
    if (this.isEmpty) return { start: new Location(0n, 0n) }

    const nextGroup = largest === undefined ? 0n : largest.group + 1n

    if (this.fieldCount === 1) {
      const delta = nextGroup - this.startGroup!
      return { start: new Location(delta < 0n ? 0n : delta > MAX_U64 ? MAX_U64 : delta, 0n) }
    }

    // Only the two-field form spells Next Object; with an end bound present, {0, 0} is
    // the absolute origin and nothing else.
    if (this.isRelativeStart) {
      const start = largest === undefined ? new Location(0n, 0n) : new Location(largest.group, largest.object + 1n)
      return { start }
    }

    const start = new Location(this.startGroup!, this.startObject!)
    if (this.endGroupDelta === undefined) return { start }
    return { start, endGroup: this.statedEndGroup, endObject: this.endObject }
  }

  /** Whether `loc` is inside the inclusive range, given the Largest Object. */
  contains(loc: Location, largest?: Location): boolean {
    const { start, endGroup, endObject } = this.resolve(largest)
    if (loc.group < start.group || (loc.group === start.group && loc.object < start.object)) return false
    if (endGroup === undefined) return true
    if (loc.group > endGroup) return false
    if (loc.group === endGroup && endObject !== undefined && loc.object > endObject) return false
    return true
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  // Byte expectations are read off §5.1.2 directly: the value is the present fields as
  // vi64s and nothing else, so a one-field filter is one byte for small values.
  describe('LocationFilter wire form', () => {
    test('the value carries exactly the present fields, in order', () => {
      expect(new LocationFilter(1n).toKeyValuePair().value).toEqual(new Uint8Array([1]))
      expect(new LocationFilter(1n, 2n).toKeyValuePair().value).toEqual(new Uint8Array([1, 2]))
      expect(new LocationFilter(1n, 2n, 3n).toKeyValuePair().value).toEqual(new Uint8Array([1, 2, 3]))
      expect(new LocationFilter(1n, 2n, 3n, 4n).toKeyValuePair().value).toEqual(new Uint8Array([1, 2, 3, 4]))
    })

    test('a zero-length value is the no-filter form, not an absent parameter', () => {
      const pair = new LocationFilter().toKeyValuePair()
      expect(pair.value).toEqual(new Uint8Array([]))
      expect(LocationFilter.fromKeyValuePair(pair)?.isEmpty).toBe(true)
    })

    test('round-trips each arity', () => {
      for (const orig of [
        new LocationFilter(),
        new LocationFilter(7n),
        new LocationFilter(7n, 8n),
        new LocationFilter(7n, 8n, 9n),
        new LocationFilter(7n, 8n, 9n, 10n),
      ]) {
        const parsed = LocationFilter.fromKeyValuePair(orig.toKeyValuePair())!
        expect([parsed.startGroup, parsed.startObject, parsed.endGroupDelta, parsed.endObject]).toEqual([
          orig.startGroup,
          orig.startObject,
          orig.endGroupDelta,
          orig.endObject,
        ])
      }
    })

    test('a field cannot be supplied without its predecessors', () => {
      expect(() => new LocationFilter(undefined, 2n)).toThrow(ProtocolViolationError)
      expect(() => new LocationFilter(1n, undefined, 3n)).toThrow(ProtocolViolationError)
    })

    test('a fifth field is refused', () => {
      const pair = KeyValuePair.tryNewBytes(LocationFilter.TYPE, new Uint8Array([1, 2, 3, 4, 5]))
      expect(() => LocationFilter.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })

    test('StartGroup + EndGroupDelta past 2^64 - 1 is refused on receipt', () => {
      const buf = new ByteBuffer()
      buf.putVI(MAX_U64)
      buf.putVI(0n)
      buf.putVI(1n)
      const pair = KeyValuePair.tryNewBytes(LocationFilter.TYPE, buf.toUint8Array())
      expect(() => LocationFilter.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })

    test('fromKeyValuePair returns undefined for another parameter type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.ObjectDeliveryTimeout, 100n)
      expect(LocationFilter.fromKeyValuePair(pair)).toBeUndefined()
    })
  })

  describe('LocationFilter resolution', () => {
    const largest = new Location(10n, 4n)

    // §5.1.2: "StartGroup=0 will start at the Next Group ... StartGroup=N will start at
    // N-1 groups prior to the current group".
    test('one field is relative to the group after Largest Object', () => {
      expect(new LocationFilter(0n).resolve(largest).start).toEqual(new Location(11n, 0n))
      expect(new LocationFilter(1n).resolve(largest).start).toEqual(new Location(10n, 0n))
      expect(new LocationFilter(2n).resolve(largest).start).toEqual(new Location(9n, 0n))
      expect(LocationFilter.nextGroup().resolve(largest).start).toEqual(new Location(11n, 0n))
      // relativeGroups(2) is 2 groups before the current one, i.e. StartGroup=3.
      expect(LocationFilter.relativeGroups(2n).resolve(largest).start).toEqual(new Location(8n, 0n))
    })

    test('a relative start before group 0 clamps to 0 rather than underflowing', () => {
      expect(new LocationFilter(99n).resolve(largest).start).toEqual(new Location(0n, 0n))
    })

    test('two zeroed fields are the Next Object, and {0, 0} on an empty track', () => {
      expect(LocationFilter.nextObject().resolve(largest).start).toEqual(new Location(10n, 5n))
      expect(LocationFilter.nextObject().resolve(undefined).start).toEqual(new Location(0n, 0n))
    })

    // The Next Object reading is two-field only: with an end bound present the same
    // zeroes are the absolute origin, which is the trap worth pinning.
    test('zeroed start fields are absolute once an end bound is present', () => {
      expect(new LocationFilter(0n, 0n, 3n).resolve(largest)).toEqual({
        start: new Location(0n, 0n),
        endGroup: 3n,
        endObject: undefined,
      })
    })

    test('two or more fields are absolute and EndGroupDelta is delta-coded', () => {
      expect(new LocationFilter(5n, 2n).resolve(largest).start).toEqual(new Location(5n, 2n))
      const ranged = LocationFilter.absoluteRange(new Location(5n, 2n), 20n, 7n)
      expect(ranged.endGroupDelta).toBe(15n)
      expect(ranged.resolve(largest)).toEqual({ start: new Location(5n, 2n), endGroup: 20n, endObject: 7n })
    })

    test('an omitted EndObject includes the whole end group', () => {
      const f = LocationFilter.absoluteRange(new Location(5n, 0n), 6n)
      expect(f.contains(new Location(6n, 999n), largest)).toBe(true)
      expect(f.contains(new Location(7n, 0n), largest)).toBe(false)
    })

    test('an omitted end is open-ended, and an empty filter passes everything', () => {
      expect(LocationFilter.absoluteStart(new Location(1n, 0n)).contains(new Location(1n << 40n, 0n))).toBe(true)
      expect(new LocationFilter().contains(new Location(0n, 0n))).toBe(true)
    })

    test('the range is inclusive at both ends', () => {
      const f = LocationFilter.absoluteRange(new Location(5n, 2n), 6n, 3n)
      expect(f.contains(new Location(5n, 1n), largest)).toBe(false)
      expect(f.contains(new Location(5n, 2n), largest)).toBe(true)
      expect(f.contains(new Location(6n, 3n), largest)).toBe(true)
      expect(f.contains(new Location(6n, 4n), largest)).toBe(false)
    })
  })
}
