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

import { ByteBuffer } from '../../common'
import { KeyValuePair } from '../../common/pair'
import { ProtocolViolationError } from '../../error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'
import type { MessageParameter } from '../message_parameter'

/** 2^64 - 1, the delta overflow boundary §5.1.4 states for Range arithmetic. */
const MAX_U64 = (1n << 64n) - 1n

/**
 * Which of the five Range Filter parameters this is (§5.1.4). The filtered
 * quantity is the only difference in meaning; the body is identical apart from
 * the Property Type the two property filters carry.
 */
export enum RangeFilterKind {
  /** §10.2.10, Subgroup ID. */
  Subgroup = MessageParameterType.SubgroupFilter,
  /** §10.2.11, Object ID. */
  ObjectId = MessageParameterType.ObjectIdFilter,
  /** §10.2.12, Publisher Priority. An 8-bit field, so a value past 255 is INVALID_FILTER. */
  Priority = MessageParameterType.PriorityFilter,
  /** §10.2.13, an Object Property named by Property Type. */
  ObjectProperty = MessageParameterType.ObjectPropertyFilter,
  /** §10.2.14, a Track Property named by Property Type. Filters whole PUBLISH messages. */
  TrackProperty = MessageParameterType.TrackPropertyFilter,
}

/** The two property filters carry a Property Type (vi64) after the SetID. */
export function hasPropertyType(kind: RangeFilterKind): boolean {
  return kind === RangeFilterKind.ObjectProperty || kind === RangeFilterKind.TrackProperty
}

/**
 * One inclusive Start/End pair. `end` is absent only on a filter's last Range,
 * where it means open-ended.
 */
export class Range {
  constructor(
    readonly start: bigint,
    readonly end?: bigint | undefined,
  ) {}

  /** The open-ended form, legal only as a filter's last Range. */
  static open(start: bigint): Range {
    return new Range(start)
  }

  contains(value: bigint): boolean {
    return value >= this.start && (this.end === undefined || value <= this.end)
  }
}

/**
 * A Range Filter parameter (§5.1.4).
 *
 * ```text
 * SUBGROUP_FILTER { Type (vi64) = 0x25, Length (vi64), [SetID (8)], [Range (..) ...] }
 * OBJECT_PROPERTY_FILTER { Type = 0x28, Length, [SetID (8)], [Property Type (vi64)], [Range ...] }
 * Range { Start (vi64), [End (vi64)] }
 * ```
 *
 * Ranges are held absolute and inclusive; the wire delta-codes them, Start from
 * the previous Range's End and End from its own Start.
 *
 * Almost every "MUST reject" in §5.1.4 is a REQUEST_ERROR with INVALID_FILTER,
 * not a session close, so it is not in the parser: {@link validateRangeFilters}
 * is the one place that decides, and it is called where a request is answered.
 */
export class RangeFilter implements Parameter {
  /**
   * `setId` undefined is the zero-length form: Length 0, no further fields, no
   * filter at all. That is a real value and not an absent parameter -- it is how
   * REQUEST_UPDATE removes a filter.
   *
   * Otherwise filters sharing a SetID are AND'd and distinct SetIDs are OR'd, so
   * the result is `SetID=0 OR SetID=1 OR ... OR SetID=255`.
   *
   * `deltaOverflow` records a Start or End delta that summed past 2^64 - 1.
   * §5.1.4 makes that a REQUEST_ERROR rather than a session close, and an
   * overflowing sum has no value to be stored in, so the parser records it,
   * keeps what it read first, and leaves the verdict to
   * {@link validateRangeFilters}.
   */
  private constructor(
    readonly kind: RangeFilterKind,
    readonly setId: number | undefined,
    readonly propertyType: bigint | undefined,
    readonly ranges: readonly Range[],
    readonly deltaOverflow: boolean,
  ) {}

  /** The zero-length form: removes a filter in REQUEST_UPDATE, filters nothing elsewhere. */
  static empty(kind: RangeFilterKind): RangeFilter {
    return new RangeFilter(kind, undefined, undefined, [], false)
  }

  /** A filter on one of the three header-field kinds. */
  static create(kind: RangeFilterKind, setId: number, ranges: readonly Range[]): RangeFilter {
    if (hasPropertyType(kind)) {
      throw new ProtocolViolationError('RangeFilter.create', `0x${kind.toString(16)} requires a Property Type`)
    }
    return RangeFilter.build(kind, setId, undefined, ranges)
  }

  /** A filter on an Object or Track Property. */
  static createProperty(
    kind: RangeFilterKind,
    setId: number,
    propertyType: bigint,
    ranges: readonly Range[],
  ): RangeFilter {
    if (!hasPropertyType(kind)) {
      throw new ProtocolViolationError('RangeFilter.createProperty', `0x${kind.toString(16)} carries no Property Type`)
    }
    return RangeFilter.build(kind, setId, propertyType, ranges)
  }

  private static build(
    kind: RangeFilterKind,
    setId: number,
    propertyType: bigint | undefined,
    ranges: readonly Range[],
  ): RangeFilter {
    if (!Number.isInteger(setId) || setId < 0 || setId > 255) {
      throw new ProtocolViolationError('RangeFilter', `SetID ${setId} is not a byte`)
    }
    // Only the last Range may be open-ended: an unbounded Range in the middle
    // leaves the next Start nothing to be a delta from.
    const open = ranges.findIndex((r) => r.end === undefined)
    if (open !== -1 && open + 1 !== ranges.length) {
      throw new ProtocolViolationError(
        'RangeFilter',
        `open-ended Range at index ${open} is not the last of ${ranges.length}`,
      )
    }
    // Start is delta-coded from the previous End and End from its own Start, and a
    // delta is unsigned, so Ranges must ascend and not overlap. Caught here, at the
    // mistake, rather than only where it would underflow on the wire.
    let prevEnd = 0n
    for (const [index, range] of ranges.entries()) {
      if (range.start < prevEnd) {
        throw new ProtocolViolationError(
          'RangeFilter',
          `Range ${index} starts at ${range.start} before the previous End ${prevEnd}`,
        )
      }
      if (range.end !== undefined) {
        if (range.end < range.start) {
          throw new ProtocolViolationError(
            'RangeFilter',
            `Range ${index} ends at ${range.end} before its own Start ${range.start}`,
          )
        }
        prevEnd = range.end
      }
    }
    return new RangeFilter(kind, setId, propertyType, [...ranges], false)
  }

  /** The zero-length form. */
  get isEmpty(): boolean {
    return this.setId === undefined
  }

  /**
   * Whether `value` falls in any Range -- Ranges are OR'd within one filter.
   *
   * An empty filter is no filter, so everything passes. A filter that states a
   * SetID and no Ranges passes nothing, which is what "falls within any Range"
   * says when there are none.
   */
  matches(value: bigint): boolean {
    if (this.isEmpty) return true
    return this.ranges.some((r) => r.contains(value))
  }

  /** The identity §5.1.4 forbids repeating within one message. */
  get dedupKey(): string {
    return `${this.kind}:${this.setId ?? '-'}:${this.propertyType ?? '-'}`
  }

  toKeyValuePair(): KeyValuePair {
    if (this.deltaOverflow) {
      throw new ProtocolViolationError('RangeFilter', 'refusing to re-emit a filter whose deltas overflowed on receipt')
    }
    const buf = new ByteBuffer()
    if (this.setId !== undefined) {
      buf.putU8(this.setId)
      if (this.propertyType !== undefined) buf.putVI(this.propertyType)
      let prevEnd = 0n
      for (const [index, range] of this.ranges.entries()) {
        if (range.start < prevEnd) {
          throw new ProtocolViolationError(
            'RangeFilter',
            `Range ${index} starts at ${range.start} before the previous End ${prevEnd}; Start is delta-coded from it`,
          )
        }
        buf.putVI(range.start - prevEnd)
        if (range.end === undefined) continue
        if (range.end < range.start) {
          throw new ProtocolViolationError(
            'RangeFilter',
            `Range ${index} ends at ${range.end} before its own Start ${range.start}`,
          )
        }
        buf.putVI(range.end - range.start)
        prevEnd = range.end
      }
    }
    // `newBytes`, not `tryNewBytes`: 0x26 and 0x28 are byte-valued on an even Type,
    // which §1.4.3's parity rule rejects. Same exception as TRACK_NAMESPACE_PREFIX.
    return KeyValuePair.newBytes(this.kind, buf.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): RangeFilter | undefined {
    const kind = rangeFilterKindOf(pair.typeValue)
    if (kind === undefined || !(pair.value instanceof Uint8Array)) return undefined
    if (pair.value.length === 0) return RangeFilter.empty(kind)

    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    const setId = buf.getU8()
    const propertyType = hasPropertyType(kind) ? buf.getVI() : undefined

    const ranges: Range[] = []
    let deltaOverflow = false
    let prevEnd = 0n
    while (buf.remaining > 0) {
      const start = prevEnd + buf.getVI()
      if (start > MAX_U64) {
        deltaOverflow = true
        break
      }
      if (buf.remaining === 0) {
        // No End left to read, so this last Range is open-ended.
        ranges.push(Range.open(start))
        break
      }
      const end = start + buf.getVI()
      if (end > MAX_U64) {
        deltaOverflow = true
        break
      }
      ranges.push(new Range(start, end))
      prevEnd = end
    }
    return new RangeFilter(kind, setId, propertyType, ranges, deltaOverflow)
  }
}

/** The Range Filter kind a parameter Type names, or undefined for any other Type. */
export function rangeFilterKindOf(typeValue: bigint | number): RangeFilterKind | undefined {
  const value = Number(typeValue)
  switch (value) {
    case MessageParameterType.SubgroupFilter:
      return RangeFilterKind.Subgroup
    case MessageParameterType.ObjectIdFilter:
      return RangeFilterKind.ObjectId
    case MessageParameterType.PriorityFilter:
      return RangeFilterKind.Priority
    case MessageParameterType.ObjectPropertyFilter:
      return RangeFilterKind.ObjectProperty
    case MessageParameterType.TrackPropertyFilter:
      return RangeFilterKind.TrackProperty
    default:
      return undefined
  }
}

/**
 * The §5.1.4 rules that need either more than one parameter or the session's
 * MAX_FILTER_RANGES to decide, plus the two per-filter rules the parser cannot
 * enforce without turning a request error into a session close.
 *
 * `maxFilterRanges` is the value **this** endpoint advertised in its Setup: the
 * option limits what the peer may send (§10.3.1.6). Its default is 0, and 0 does
 * not mean unlimited -- it prohibits Range Filters outright, which is why an
 * endpoint that never sends the option must reject every one it receives.
 *
 * Returns the reason the request must be answered with REQUEST_ERROR carrying
 * INVALID_FILTER, or undefined when the filters are acceptable.
 */
export function validateRangeFilters(params: readonly MessageParameter[], maxFilterRanges: bigint): string | undefined {
  const filters = params.filter((p): p is RangeFilter => p instanceof RangeFilter)
  if (filters.length === 0) return undefined
  if (maxFilterRanges === 0n) return 'MAX_FILTER_RANGES is 0, so no Range Filter parameter may be sent'

  let totalRanges = 0n
  const seen = new Set<string>()
  for (const filter of filters) {
    const type = `0x${filter.kind.toString(16)}`
    if (filter.deltaOverflow) return `a Range delta in parameter ${type} sums past 2^64 - 1`
    if (seen.has(filter.dedupKey)) {
      return `parameter ${type} repeats SetID ${filter.setId} and Property Type ${filter.propertyType}`
    }
    seen.add(filter.dedupKey)
    if (filter.propertyType !== undefined && filter.propertyType % 2n !== 0n) {
      return `Property Type ${filter.propertyType} is odd, so it is not a single integer value`
    }
    if (filter.kind === RangeFilterKind.Priority) {
      for (const range of filter.ranges) {
        for (const value of [range.start, range.end]) {
          if (value !== undefined && value > 255n) {
            return `PRIORITY_FILTER value ${value} exceeds 255; Publisher Priority is an 8-bit field`
          }
        }
      }
    }
    totalRanges += BigInt(filter.ranges.length)
  }
  if (totalRanges > maxFilterRanges) {
    return `${totalRanges} Ranges exceed the advertised MAX_FILTER_RANGES of ${maxFilterRanges}`
  }
  return undefined
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  const filter = (ranges: Range[]) => RangeFilter.create(RangeFilterKind.Subgroup, 0, ranges)
  const parse = (kind: RangeFilterKind, bytes: Uint8Array) =>
    RangeFilter.fromKeyValuePair(KeyValuePair.newBytes(kind, bytes))!

  describe('RangeFilter wire form', () => {
    // The draft's own worked example: "ranges 3-5 and 10-15 encode as: Start=3,
    // End=2, Start=5, End=5".
    test('the draft worked example encodes byte for byte', () => {
      const f = filter([new Range(3n, 5n), new Range(10n, 15n)])
      expect(f.toKeyValuePair().value).toEqual(new Uint8Array([0, 3, 2, 5, 5]))
      expect(RangeFilter.fromKeyValuePair(f.toKeyValuePair())).toEqual(f)
    })

    test('a zero-length value is the no-filter form, not an absent parameter', () => {
      const empty = RangeFilter.empty(RangeFilterKind.ObjectId)
      expect(empty.toKeyValuePair().value).toEqual(new Uint8Array([]))
      expect(RangeFilter.fromKeyValuePair(empty.toKeyValuePair())?.isEmpty).toBe(true)
      // No filter passes everything; a stated filter with no Ranges passes nothing.
      expect(empty.matches(7n)).toBe(true)
      expect(filter([]).matches(7n)).toBe(false)
    })

    test('the last Range may be open-ended and an earlier one may not', () => {
      const f = filter([new Range(1n, 2n), Range.open(9n)])
      expect(f.toKeyValuePair().value).toEqual(new Uint8Array([0, 1, 1, 7]))
      const parsed = RangeFilter.fromKeyValuePair(f.toKeyValuePair())!
      expect(parsed).toEqual(f)
      expect(parsed.matches(MAX_U64)).toBe(true)
      expect(() => filter([Range.open(1n), new Range(5n, 6n)])).toThrow(ProtocolViolationError)
    })

    test('Ranges must ascend and not overlap', () => {
      expect(() => filter([new Range(10n, 15n), new Range(3n, 5n)])).toThrow(ProtocolViolationError)
      expect(() => filter([new Range(3n, 9n), new Range(5n, 11n)])).toThrow(ProtocolViolationError)
      expect(() => filter([new Range(5n, 3n)])).toThrow(ProtocolViolationError)
    })

    test('Ranges are OR-d within a filter and inclusive at both ends', () => {
      const f = filter([new Range(3n, 5n), new Range(10n, 15n)])
      expect([2n, 3n, 5n, 6n, 10n, 15n, 16n].map((v) => f.matches(v))).toEqual([
        false,
        true,
        true,
        false,
        true,
        true,
        false,
      ])
    })

    test('a property filter carries its Property Type between SetID and Ranges', () => {
      const f = RangeFilter.createProperty(RangeFilterKind.ObjectProperty, 7, 4n, [new Range(1n, 1n)])
      expect(f.toKeyValuePair().value).toEqual(new Uint8Array([7, 4, 1, 0]))
      expect(RangeFilter.fromKeyValuePair(f.toKeyValuePair())).toEqual(f)
      expect(() => RangeFilter.create(RangeFilterKind.ObjectProperty, 0, [])).toThrow(ProtocolViolationError)
      expect(() => RangeFilter.createProperty(RangeFilterKind.Subgroup, 0, 2n, [])).toThrow(ProtocolViolationError)
    })

    // The Property Type is read only for 0x28/0x29, so the same bytes mean
    // different things under the two shapes: the 4 that was a Property Type
    // becomes the first Start and everything after shifts by one field. Reading
    // one as the other is the desync this pins.
    test('the Property Type is not read for a header-field filter', () => {
      const asSubgroup = parse(RangeFilterKind.Subgroup, new Uint8Array([7, 4, 1, 0]))
      expect(asSubgroup.propertyType).toBeUndefined()
      expect(asSubgroup.ranges).toEqual([new Range(4n, 5n), Range.open(5n)])
    })

    test('fromKeyValuePair returns undefined for another parameter type', () => {
      expect(RangeFilter.fromKeyValuePair(KeyValuePair.tryNewVarInt(MessageParameterType.Expires, 1n))).toBeUndefined()
    })
  })

  describe('validateRangeFilters', () => {
    const wrap = (...fs: RangeFilter[]) => fs as unknown as MessageParameter[]

    // Both sums can overflow and they are separate adds: End is Start plus its own
    // delta, Start is the previous End plus its own. A test that only reaches one
    // leaves the other free to go unnoticed.
    test('either delta summing past 2^64 - 1 is recorded rather than closing the session', () => {
      const bytes = (...deltas: bigint[]) => {
        const buf = new ByteBuffer()
        buf.putU8(0)
        for (const d of deltas) buf.putVI(d)
        return buf.toUint8Array()
      }
      // End overflows: Start is already 2^64 - 1. Then Start overflows: the first
      // Range ends there, so the next Start cannot be a non-zero delta from it.
      for (const value of [bytes(MAX_U64, 1n), bytes(MAX_U64, 0n, 1n)]) {
        const parsed = parse(RangeFilterKind.Subgroup, value)
        expect(parsed.deltaOverflow).toBe(true)
        expect(() => parsed.toKeyValuePair()).toThrow(ProtocolViolationError)
        expect(validateRangeFilters(wrap(parsed), 16n)).toBe('a Range delta in parameter 0x25 sums past 2^64 - 1')
      }
    })

    test('MAX_FILTER_RANGES of 0 prohibits Range Filters outright', () => {
      const params = wrap(filter([new Range(1n, 2n)]))
      expect(validateRangeFilters(params, 0n)).toBeDefined()
      expect(validateRangeFilters(params, 1n)).toBeUndefined()
      expect(validateRangeFilters([], 0n)).toBeUndefined()
      // Even the zero-length removal form is "such a filter parameter".
      expect(validateRangeFilters(wrap(RangeFilter.empty(RangeFilterKind.Subgroup)), 0n)).toBeDefined()
    })

    test('the limit counts Ranges across every filter in the request', () => {
      const params = wrap(
        filter([new Range(1n, 2n), new Range(4n, 5n)]),
        RangeFilter.create(RangeFilterKind.ObjectId, 0, [new Range(1n, 2n)]),
      )
      expect(validateRangeFilters(params, 3n)).toBeUndefined()
      expect(validateRangeFilters(params, 2n)).toBeDefined()
    })

    test('repeating Type, SetID and Property Type is refused', () => {
      const one = RangeFilter.createProperty(RangeFilterKind.TrackProperty, 1, 2n, [])
      const sameKey = RangeFilter.createProperty(RangeFilterKind.TrackProperty, 1, 2n, [])
      expect(validateRangeFilters(wrap(one, sameKey), 16n)).toBeDefined()
      // Repetition is allowed; it is this exact combination that is not.
      const otherSet = RangeFilter.createProperty(RangeFilterKind.TrackProperty, 2, 2n, [])
      const otherProperty = RangeFilter.createProperty(RangeFilterKind.TrackProperty, 1, 4n, [])
      expect(validateRangeFilters(wrap(one, otherSet, otherProperty), 16n)).toBeUndefined()
    })

    test('an odd Property Type is refused because it is not a single integer', () => {
      const odd = RangeFilter.createProperty(RangeFilterKind.ObjectProperty, 0, 3n, [])
      expect(validateRangeFilters(wrap(odd), 16n)).toBeDefined()
      const even = RangeFilter.createProperty(RangeFilterKind.ObjectProperty, 0, 4n, [])
      expect(validateRangeFilters(wrap(even), 16n)).toBeUndefined()
    })

    test('a priority past 255 is refused, and only for the priority filter', () => {
      const over = RangeFilter.create(RangeFilterKind.Priority, 0, [new Range(250n, 300n)])
      expect(validateRangeFilters(wrap(over), 16n)).toBeDefined()
      const open = RangeFilter.create(RangeFilterKind.Priority, 0, [Range.open(256n)])
      expect(validateRangeFilters(wrap(open), 16n)).toBeDefined()
      const within = RangeFilter.create(RangeFilterKind.Priority, 0, [new Range(0n, 255n)])
      expect(validateRangeFilters(wrap(within), 16n)).toBeUndefined()
      // The same values are ordinary Subgroup IDs.
      expect(validateRangeFilters(wrap(filter([new Range(250n, 300n)])), 16n)).toBeUndefined()
    })
  })
}
