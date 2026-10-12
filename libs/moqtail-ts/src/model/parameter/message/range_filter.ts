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

import { ByteBuffer } from '../../common/byte_buffer'
import { KeyValuePair } from '../../common/pair'
import { MOQtailError, ProtocolViolationError } from '../../error/error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'

const MAX_U64 = 0xffff_ffff_ffff_ffffn

/**
 * What a Range Filter selects on. Each kind is its own parameter type, and the two
 * property kinds also name the Property they read.
 */
export enum RangeFilterKind {
  Subgroup = MessageParameterType.SubgroupFilter,
  ObjectId = MessageParameterType.ObjectIdFilter,
  /** Publisher Priority, which is a single byte. */
  Priority = MessageParameterType.PriorityFilter,
  ObjectProperty = MessageParameterType.ObjectPropertyFilter,
  /** Selects tracks rather than Objects, so it only rides SUBSCRIBE_TRACKS. */
  TrackProperty = MessageParameterType.TrackPropertyFilter,
}

export namespace RangeFilterKind {
  /** Whether the filter carries a Property Type after its SetID. */
  export function hasPropertyType(kind: RangeFilterKind): boolean {
    return kind === RangeFilterKind.ObjectProperty || kind === RangeFilterKind.TrackProperty
  }

  export function isRangeFilterType(typeValue: bigint | number): boolean {
    const n = Number(typeValue)
    return n >= MessageParameterType.SubgroupFilter && n <= MessageParameterType.TrackPropertyFilter
  }
}

/** An inclusive range of values. An open-ended range has no `end`. */
export interface Range {
  readonly start: bigint
  readonly end?: bigint
}

/**
 * A Range as the wire carries it: the Start as a delta from the End before it, the
 * End as a delta from its own Start.
 */
export type RangeDelta = readonly [start: bigint, end?: bigint]

/**
 * A Range Filter that cannot be honoured. A peer that sends one is answered with
 * REQUEST_ERROR INVALID_FILTER rather than a closed session: the request is
 * well-formed, it just asks for something that cannot be applied.
 */
export class InvalidFilterError extends MOQtailError {
  constructor(
    public readonly context: string,
    public readonly details: string,
  ) {
    super(`Invalid filter in ${context}: ${details}`)
  }
}

/**
 * Delta-encodes ascending `ranges`. Only the last may be open-ended.
 * @throws :{@link InvalidFilterError} When the ranges do not ascend.
 */
export function encodeRanges(ranges: readonly Range[]): RangeDelta[] {
  const deltas: RangeDelta[] = []
  let priorEnd = 0n
  ranges.forEach((range, i) => {
    if (range.start < priorEnd || (range.end !== undefined && range.end < range.start)) {
      throw new InvalidFilterError('encodeRanges', 'ranges are not ascending')
    }
    if (range.end === undefined) {
      if (i + 1 !== ranges.length) throw new InvalidFilterError('encodeRanges', 'only the last range can be open-ended')
      deltas.push([range.start - priorEnd])
      return
    }
    deltas.push([range.start - priorEnd, range.end - range.start])
    priorEnd = range.end
  })
  return deltas
}

/**
 * One Range Filter parameter. Unlike other parameters it may repeat in a message,
 * once per SetID (and Property Type).
 *
 * Without a `setId` it is the zero-length form, which in REQUEST_UPDATE removes the
 * filters of its kind.
 *
 * The Ranges are held as the wire's deltas and only added back by {@link
 * RangeFilter.ranges}: a sum past 2^64-1 is answered with a REQUEST_ERROR for the
 * request that carried it, which a parser that rejected it could not do.
 */
export class RangeFilter implements Parameter {
  /**
   * @throws :{@link ProtocolViolationError} When the Property Type's presence does not
   * match the kind, or a removal carries anything but its kind.
   */
  constructor(
    public readonly kind: RangeFilterKind,
    public readonly setId?: number,
    public readonly propertyType?: bigint,
    public readonly deltas: readonly RangeDelta[] = [],
  ) {
    if (setId === undefined) {
      if (propertyType !== undefined || deltas.length > 0) {
        throw new ProtocolViolationError('RangeFilter', 'a removal carries no SetID, Property Type or Ranges')
      }
      return
    }
    if (!Number.isInteger(setId) || setId < 0 || setId > 255) {
      throw new ProtocolViolationError('RangeFilter', `SetID ${setId} is not a single byte`)
    }
    // A Property Type where the kind has none, or none where it has one, would be
    // read back as a Range.
    if (RangeFilterKind.hasPropertyType(kind) !== (propertyType !== undefined)) {
      throw new ProtocolViolationError(
        'RangeFilter',
        `${RangeFilterKind[kind]} filter has the wrong Property Type presence`,
      )
    }
    deltas.forEach(([, end], i) => {
      if (end === undefined && i + 1 !== deltas.length) {
        throw new ProtocolViolationError('RangeFilter', 'only the last range can be open-ended')
      }
    })
  }

  /** The zero-length form, which removes the filters of its kind. */
  get isRemoval(): boolean {
    return this.setId === undefined
  }

  /** How many Ranges the filter holds, which is what a peer's range limit counts. */
  get rangeCount(): number {
    return this.deltas.length
  }

  /**
   * The Ranges with their deltas added back.
   * @throws :{@link InvalidFilterError} When a bound exceeds 2^64-1.
   */
  ranges(): Range[] {
    const ranges: Range[] = []
    let priorEnd = 0n
    for (const [startDelta, endDelta] of this.deltas) {
      const start = priorEnd + startDelta
      if (start > MAX_U64) throw new InvalidFilterError('RangeFilter.ranges', 'a range bound exceeds 2^64 - 1')
      if (endDelta === undefined) {
        ranges.push({ start })
        continue
      }
      const end = start + endDelta
      if (end > MAX_U64) throw new InvalidFilterError('RangeFilter.ranges', 'a range bound exceeds 2^64 - 1')
      ranges.push({ start, end })
      priorEnd = end
    }
    return ranges
  }

  /**
   * The Ranges, once everything a filter of this kind has to satisfy holds.
   * @throws :{@link InvalidFilterError} For an odd Property Type, a priority above
   * 255, or a bound past 2^64-1.
   */
  validate(): Range[] {
    if (this.propertyType !== undefined && this.propertyType % 2n !== 0n) {
      throw new InvalidFilterError('RangeFilter.validate', `property type 0x${this.propertyType.toString(16)} is odd`)
    }
    const ranges = this.ranges()
    if (this.kind === RangeFilterKind.Priority) {
      const over = ranges.flatMap((r) => [r.start, r.end]).find((v) => v !== undefined && v > 255n)
      if (over !== undefined) throw new InvalidFilterError('RangeFilter.validate', `priority ${over} is outside 0-255`)
    }
    return ranges
  }

  /** The SetID, Property Type if any, and the Ranges, without the Type or Length. */
  serializeValue(): Uint8Array {
    if (this.setId === undefined) return new Uint8Array()
    const buf = new ByteBuffer()
    buf.putU8(this.setId)
    if (this.propertyType !== undefined) buf.putVI(this.propertyType)
    for (const [start, end] of this.deltas) {
      buf.putVI(start)
      if (end !== undefined) buf.putVI(end)
    }
    return buf.toUint8Array()
  }

  // Length-prefixed whatever the Type's parity.
  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.newBytes(this.kind, this.serializeValue())
  }

  /**
   * Decodes a Range Filter value. The Value ends with the last Range, so a Start
   * with nothing after it is an open-ended last Range.
   */
  static fromKeyValuePair(pair: KeyValuePair): RangeFilter | undefined {
    if (!RangeFilterKind.isRangeFilterType(pair.typeValue) || !(pair.value instanceof Uint8Array)) return undefined
    const kind = Number(pair.typeValue) as RangeFilterKind
    if (pair.value.length === 0) return new RangeFilter(kind)

    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    const frozen = buf.freeze()
    const setId = frozen.getU8()
    const propertyType = RangeFilterKind.hasPropertyType(kind) ? frozen.getVI() : undefined
    const deltas: RangeDelta[] = []
    while (frozen.remaining > 0) {
      const start = frozen.getVI()
      deltas.push(frozen.remaining > 0 ? [start, frozen.getVI()] : [start])
    }
    return new RangeFilter(kind, setId, propertyType, deltas)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { NotEnoughBytesError } = await import('../../error/error')

  const roundtrip = (filter: RangeFilter) => {
    expect(RangeFilter.fromKeyValuePair(filter.toKeyValuePair())).toEqual(filter)
  }

  describe('RangeFilter', () => {
    test('ranges are delta-encoded from the prior end', () => {
      // 3-5 and 10-15 go on the wire as Start=3, End=2, Start=5, End=5.
      const deltas = encodeRanges([
        { start: 3n, end: 5n },
        { start: 10n, end: 15n },
      ])
      expect(deltas).toEqual([
        [3n, 2n],
        [5n, 5n],
      ])
      const filter = new RangeFilter(RangeFilterKind.Subgroup, 0, undefined, deltas)
      expect(filter.serializeValue()).toEqual(new Uint8Array([0x00, 3, 2, 5, 5]))
      expect(filter.ranges()).toEqual([
        { start: 3n, end: 5n },
        { start: 10n, end: 15n },
      ])
    })

    test('a start with nothing after it is open-ended', () => {
      const pair = KeyValuePair.newBytes(RangeFilterKind.Subgroup, new Uint8Array([0x07, 3, 2, 5]))
      const filter = RangeFilter.fromKeyValuePair(pair)!
      expect(filter.setId).toBe(7)
      expect(filter.ranges()).toEqual([{ start: 3n, end: 5n }, { start: 10n }])
    })

    test('a property filter names its property after the set id', () => {
      const filter = new RangeFilter(RangeFilterKind.ObjectProperty, 2, 0x3cn, encodeRanges([{ start: 10n, end: 20n }]))
      expect(filter.serializeValue()).toEqual(new Uint8Array([0x02, 0x3c, 10, 10]))
      roundtrip(filter)
    })

    test('every kind roundtrips, including the removal', () => {
      for (const kind of [
        RangeFilterKind.Subgroup,
        RangeFilterKind.ObjectId,
        RangeFilterKind.Priority,
        RangeFilterKind.ObjectProperty,
        RangeFilterKind.TrackProperty,
      ]) {
        const propertyType = RangeFilterKind.hasPropertyType(kind) ? 0x40n : undefined
        roundtrip(new RangeFilter(kind, 1, propertyType, encodeRanges([{ start: 0n, end: 0n }, { start: 9n }])))
        roundtrip(new RangeFilter(kind))
      }
    })

    test('a missing property type is malformed', () => {
      const pair = KeyValuePair.newBytes(RangeFilterKind.TrackProperty, new Uint8Array([0x00]))
      expect(() => RangeFilter.fromKeyValuePair(pair)).toThrow(NotEnoughBytesError)
    })

    test('the property type must be present exactly for property filters', () => {
      expect(() => new RangeFilter(RangeFilterKind.ObjectProperty, 0)).toThrow(ProtocolViolationError)
      expect(() => new RangeFilter(RangeFilterKind.Subgroup, 0, 0x3cn)).toThrow(ProtocolViolationError)
    })

    test('an overflowing delta is rejected when resolved, not when parsed', () => {
      const filter = new RangeFilter(RangeFilterKind.ObjectId, 0, undefined, [[MAX_U64, 1n]])
      expect(RangeFilter.fromKeyValuePair(filter.toKeyValuePair())).toEqual(filter)
      expect(() => filter.ranges()).toThrow(InvalidFilterError)
    })

    test('a priority above 255 is rejected', () => {
      const deltas = encodeRanges([{ start: 10n, end: 256n }])
      expect(() => new RangeFilter(RangeFilterKind.Priority, 0, undefined, deltas).validate()).toThrow(
        InvalidFilterError,
      )
      expect(new RangeFilter(RangeFilterKind.ObjectId, 0, undefined, deltas).validate()).toHaveLength(1)
      // An open end does not name a value above 255.
      const open = new RangeFilter(RangeFilterKind.Priority, 0, undefined, encodeRanges([{ start: 200n }]))
      expect(open.validate()).toEqual([{ start: 200n }])
    })

    test('an odd property type is rejected', () => {
      const filter = new RangeFilter(RangeFilterKind.ObjectProperty, 0, 0x3dn, encodeRanges([{ start: 1n, end: 1n }]))
      expect(() => filter.validate()).toThrow(InvalidFilterError)
    })

    test('descending or mid-list open ranges cannot be encoded', () => {
      expect(() =>
        encodeRanges([
          { start: 10n, end: 15n },
          { start: 3n, end: 5n },
        ]),
      ).toThrow(InvalidFilterError)
      expect(() => encodeRanges([{ start: 5n, end: 3n }])).toThrow(InvalidFilterError)
      expect(() => encodeRanges([{ start: 3n }, { start: 10n, end: 15n }])).toThrow(InvalidFilterError)
    })
  })
}
