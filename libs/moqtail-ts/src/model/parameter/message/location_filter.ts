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

import { BaseByteBuffer, ByteBuffer } from '../../common/byte_buffer'
import { KeyValuePair } from '../../common/pair'
import { Location } from '../../common/location'
import { ProtocolViolationError } from '../../error/error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'
import { LocationFilterType } from '../../control/constant'

const MAX_U64 = 0xffff_ffff_ffff_ffffn

/** The fields each Location Filter Type carries, in wire order. */
const FIELD_COUNT: Record<LocationFilterType, number> = {
  [LocationFilterType.None]: 0,
  [LocationFilterType.RelativeStart]: 1,
  [LocationFilterType.AbsoluteStart]: 2,
  [LocationFilterType.AbsoluteStartGroupEnd]: 3,
  [LocationFilterType.AbsoluteRange]: 4,
  [LocationFilterType.NextObject]: 0,
}

/**
 * The range of Locations a subscription or fetch selects. The Location Filter
 * Type decides which fields are set, and the value is not length-prefixed.
 *
 * Every Group here is absolute, except `startGroup` of a
 * {@link (LocationFilterType:enum).RelativeStart}, which counts back from the Next
 * Group: 0 is the Next Group, 1 the current one. The wire carries the End Group
 * as a delta from the Start Group; that is undone on the way in and redone on
 * the way out.
 *
 * If omitted from SUBSCRIBE, the subscription is unfiltered. If omitted from
 * REQUEST_UPDATE, the value is unchanged.
 */
export class LocationFilter implements Parameter {
  static readonly TYPE = MessageParameterType.LocationFilter

  /**
   * @throws :{@link ProtocolViolationError} When a field the type calls for is
   * missing, or the End Group is before the Start Group.
   */
  constructor(
    public readonly filterType: LocationFilterType,
    public readonly startGroup?: bigint,
    public readonly startObject?: bigint,
    public readonly endGroup?: bigint,
    public readonly endObject?: bigint,
  ) {
    const fields = [startGroup, startObject, endGroup, endObject]
    const count = FIELD_COUNT[filterType]
    if (count === undefined) {
      throw new ProtocolViolationError('LocationFilter', `unknown Location Filter Type ${filterType}`)
    }
    fields.forEach((field, i) => {
      if ((field !== undefined) !== i < count) {
        throw new ProtocolViolationError(
          'LocationFilter',
          `Location Filter Type ${filterType} carries ${count} fields; field ${i + 1} is ${field === undefined ? 'missing' : 'not one of them'}`,
        )
      }
    })
    if (endGroup !== undefined && endGroup < startGroup!) {
      throw new ProtocolViolationError('LocationFilter', `End Group ${endGroup} is before Start Group ${startGroup}`)
    }
  }

  /**
   * The first Location the filter passes, given the track's Largest Object, or
   * `undefined` when there is no filter. A relative start that would fall before
   * group 0 is group 0; a track with nothing published yet starts a relative
   * filter at `{0,0}`.
   */
  startLocation(largest?: Location): Location | undefined {
    switch (this.filterType) {
      case LocationFilterType.None:
        return undefined
      case LocationFilterType.RelativeStart: {
        if (!largest) return new Location(0n, 0n)
        const group = largest.group + 1n - this.startGroup!
        return new Location(group < 0n ? 0n : group > MAX_U64 ? MAX_U64 : group, 0n)
      }
      case LocationFilterType.NextObject:
        return largest ? new Location(largest.group, largest.object + 1n) : new Location(0n, 0n)
      default:
        return new Location(this.startGroup!, this.startObject!)
    }
  }

  /** The Start Location an absolute filter carries; `undefined` for the others. */
  get absoluteStart(): Location | undefined {
    switch (this.filterType) {
      case LocationFilterType.AbsoluteStart:
      case LocationFilterType.AbsoluteStartGroupEnd:
      case LocationFilterType.AbsoluteRange:
        return new Location(this.startGroup!, this.startObject!)
      default:
        return undefined
    }
  }

  /**
   * The last Location the filter passes, inclusive, or `undefined` when it is
   * open-ended. A Group End is the whole of that group, so its Object is 2^64-1.
   */
  endLocation(): Location | undefined {
    switch (this.filterType) {
      case LocationFilterType.AbsoluteStartGroupEnd:
        return new Location(this.endGroup!, MAX_U64)
      case LocationFilterType.AbsoluteRange:
        return new Location(this.endGroup!, this.endObject!)
      default:
        return undefined
    }
  }

  /** The Location Filter Type and its fields, without the Parameter Type. */
  serializeValue(): Uint8Array {
    const buf = new ByteBuffer()
    buf.putVI(this.filterType)
    const count = FIELD_COUNT[this.filterType]
    if (count > 0) buf.putVI(this.startGroup!)
    if (count > 1) buf.putVI(this.startObject!)
    if (count > 2) buf.putVI(this.endGroup! - this.startGroup!)
    if (count > 3) buf.putVI(this.endObject!)
    return buf.toUint8Array()
  }

  /**
   * Reads a Location Filter Type and exactly the fields it calls for.
   * @throws :{@link ProtocolViolationError} For an unknown Location Filter Type,
   * or an End Group past 2^64-1.
   */
  static deserializeValue(buf: BaseByteBuffer): LocationFilter {
    const raw = buf.getVI()
    let filterType: LocationFilterType
    try {
      filterType = LocationFilterType.tryFrom(raw)
    } catch {
      throw new ProtocolViolationError('LocationFilter.deserializeValue', `unknown Location Filter Type ${raw}`)
    }
    const count = FIELD_COUNT[filterType]
    const startGroup = count > 0 ? buf.getVI() : undefined
    const startObject = count > 1 ? buf.getVI() : undefined
    let endGroup: bigint | undefined
    if (count > 2) {
      endGroup = startGroup! + buf.getVI()
      if (endGroup > MAX_U64) {
        throw new ProtocolViolationError(
          'LocationFilter.deserializeValue',
          `StartGroup plus EndGroupDelta exceeds 2^64 - 1`,
        )
      }
    }
    const endObject = count > 3 ? buf.getVI() : undefined
    return new LocationFilter(filterType, startGroup, startObject, endGroup, endObject)
  }

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewBytes(LocationFilter.TYPE, this.serializeValue())
  }

  static fromKeyValuePair(pair: KeyValuePair): LocationFilter | undefined {
    if (Number(pair.typeValue) !== LocationFilter.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    const frozen = buf.freeze()
    const filter = LocationFilter.deserializeValue(frozen)
    if (frozen.remaining > 0) {
      throw new ProtocolViolationError('LocationFilter.fromKeyValuePair', 'trailing bytes after the Location Filter')
    }
    return filter
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  const decode = (bytes: number[]) => {
    const buf = new ByteBuffer()
    buf.putBytes(new Uint8Array(bytes))
    return LocationFilter.deserializeValue(buf.freeze())
  }

  describe('LocationFilter', () => {
    test('every filter type roundtrips', () => {
      for (const filter of [
        new LocationFilter(LocationFilterType.None),
        new LocationFilter(LocationFilterType.RelativeStart, 3n),
        new LocationFilter(LocationFilterType.AbsoluteStart, 10n, 2n),
        new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 10n, 2n, 15n),
        new LocationFilter(LocationFilterType.AbsoluteRange, 10n, 2n, 15n, 7n),
        new LocationFilter(LocationFilterType.NextObject),
      ]) {
        expect(LocationFilter.fromKeyValuePair(filter.toKeyValuePair())).toEqual(filter)
      }
    })

    test('the type decides which fields follow', () => {
      const cases: [LocationFilter, number[]][] = [
        [new LocationFilter(LocationFilterType.None), [0x00]],
        [new LocationFilter(LocationFilterType.RelativeStart, 1n), [0x01, 0x01]],
        [new LocationFilter(LocationFilterType.AbsoluteStart, 5n, 2n), [0x02, 0x05, 0x02]],
        [new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 5n, 2n, 8n), [0x03, 0x05, 0x02, 0x03]],
        [new LocationFilter(LocationFilterType.AbsoluteRange, 5n, 2n, 8n, 4n), [0x04, 0x05, 0x02, 0x03, 0x04]],
        [new LocationFilter(LocationFilterType.NextObject), [0x05]],
      ]
      for (const [filter, wire] of cases) {
        expect(filter.serializeValue()).toEqual(new Uint8Array(wire))
        expect(decode(wire)).toEqual(filter)
      }
    })

    test('decoding reads only the fields its type calls for', () => {
      const buf = new ByteBuffer()
      buf.putBytes(new Uint8Array([0x05, 0x10, 0x01]))
      const frozen = buf.freeze()
      expect(LocationFilter.deserializeValue(frozen)).toEqual(new LocationFilter(LocationFilterType.NextObject))
      expect(frozen.remaining).toBe(2)
    })

    test('an unknown filter type is a protocol violation', () => {
      expect(() => decode([0x06])).toThrow(ProtocolViolationError)
    })

    test('the end group is the start group plus the delta', () => {
      // Both varints at their largest still sum to less than 2^64.
      const buf = new ByteBuffer()
      buf.putVI(LocationFilterType.AbsoluteStartGroupEnd)
      buf.putVI(2n ** 62n - 1n)
      buf.putVI(0n)
      buf.putVI(2n ** 62n - 1n)
      const frozen = buf.freeze()
      expect(LocationFilter.deserializeValue(frozen).endGroup).toBe(2n ** 63n - 2n)
    })

    test('a truncated filter is an error', () => {
      expect(() => decode([0x04, 0x05, 0x02, 0x03])).toThrow()
    })

    test('a missing or extra field is refused', () => {
      expect(() => new LocationFilter(LocationFilterType.AbsoluteStart, 5n)).toThrow(ProtocolViolationError)
      expect(() => new LocationFilter(LocationFilterType.NextObject, 5n)).toThrow(ProtocolViolationError)
    })

    test('an end group before the start group is refused', () => {
      expect(() => new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 10n, 0n, 9n)).toThrow(
        ProtocolViolationError,
      )
    })

    test('a relative start counts back from the next group', () => {
      const largest = new Location(7n, 3n)
      const start = (n: bigint) => new LocationFilter(LocationFilterType.RelativeStart, n).startLocation(largest)
      expect(start(0n)).toEqual(new Location(8n, 0n))
      expect(start(1n)).toEqual(new Location(7n, 0n))
      expect(start(2n)).toEqual(new Location(6n, 0n))
      expect(start(100n)).toEqual(new Location(0n, 0n))
    })

    test('next object starts after the largest object', () => {
      expect(new LocationFilter(LocationFilterType.NextObject).startLocation(new Location(7n, 3n))).toEqual(
        new Location(7n, 4n),
      )
      expect(new LocationFilter(LocationFilterType.NextObject).startLocation()).toEqual(new Location(0n, 0n))
    })

    test('no filter has no start and no end', () => {
      const none = new LocationFilter(LocationFilterType.None)
      expect(none.startLocation(new Location(7n, 3n))).toBeUndefined()
      expect(none.endLocation()).toBeUndefined()
    })

    test('only an absolute filter carries its own start', () => {
      expect(new LocationFilter(LocationFilterType.AbsoluteStart, 2n, 1n).absoluteStart).toEqual(new Location(2n, 1n))
      expect(new LocationFilter(LocationFilterType.RelativeStart, 2n).absoluteStart).toBeUndefined()
      expect(new LocationFilter(LocationFilterType.NextObject).absoluteStart).toBeUndefined()
    })

    test('a group end covers the whole of its last group', () => {
      expect(new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 2n, 0n, 4n).endLocation()).toEqual(
        new Location(4n, MAX_U64),
      )
      expect(new LocationFilter(LocationFilterType.AbsoluteRange, 2n, 0n, 4n, 6n).endLocation()).toEqual(
        new Location(4n, 6n),
      )
    })

    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.ObjectDeliveryTimeout, 100n)
      expect(LocationFilter.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
