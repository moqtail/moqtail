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

import { Location } from './common/location'
import type { KeyValuePair } from './common/pair'
import type { Tuple } from './common/tuple'
import { LocationFilterType } from './constants'
import type { GroupOrder, SwitchMode, TokenAliasType } from './constants'
import { MOQtailError, ProtocolViolationError } from './error/error'

/**
 * @public
 * Which Objects a request selects (moqt-22 §9.2.2.5). The fields present depend on
 * `filterType`: RELATIVE_START carries `startGroup` only (groups back from the Largest
 * Object), the absolute types a start location, and the two bounded ones an end group,
 * plus an end object for ABSOLUTE_RANGE. Build one with {@link locationFilter}.
 */
export interface LocationFilter {
  readonly filterType: LocationFilterType
  readonly startGroup?: bigint
  readonly startObject?: bigint
  readonly endGroup?: bigint
  readonly endObject?: bigint
}

/**
 * @public
 * One Range Filter (moqt-22 §9.2.2.8): the ranges of one SetID, delta-coded. A filter
 * with no `setId` is a removal: carried in a REQUEST_UPDATE, it drops the filters of its
 * kind.
 */
export interface RangeFilter {
  readonly setId?: number
  /** The property the ranges apply to, for the property filters only. */
  readonly propertyType?: bigint
  readonly deltas: readonly RangeDelta[]
}

/** @public A range as coded: its start after the previous end, and its length; open-ended when last. */
export type RangeDelta = readonly [start: bigint, end?: bigint]

/** @public A decoded range: `end` is inclusive, and absent on an open-ended last range. */
export interface Range {
  readonly start: bigint
  readonly end?: bigint
}

/** @public The parameters that hold Range Filters, one list per filter kind. */
export type RangeFilterKey =
  'subgroupFilter' | 'objectIdFilter' | 'priorityFilter' | 'objectPropertyFilter' | 'trackPropertyFilter'

/** @public SWITCH_FROM (moqt-22 §9.2.2.12): the subscription this one takes over from. */
export interface SwitchFrom {
  readonly requestId: bigint
  readonly mode: SwitchMode
  /** Whether the publisher sends PUBLISH_DONE on the old subscription once it drains. */
  readonly publishDone: boolean
}

/** @public SWITCHING_SET_ASSIGNMENT (SSTS 5.1): the switching set a track belongs to. */
export interface SwitchingSetAssignment {
  readonly switchingSetId: bigint
  readonly algorithmId: bigint
  readonly throughputThresholdKbps: bigint
  /** 1 to 10. */
  readonly setThroughputWeight: bigint
  readonly activateSwitching: bigint
  readonly setRank: number
}

/** @public AUTHORIZATION TOKEN (moqt-22 §9.2.1.1), by alias type. */
export type AuthorizationToken =
  | { readonly aliasType: TokenAliasType.Delete; readonly tokenAlias: bigint }
  | {
      readonly aliasType: TokenAliasType.Register
      readonly tokenAlias: bigint
      readonly tokenType: bigint
      readonly tokenValue: Uint8Array
    }
  | { readonly aliasType: TokenAliasType.UseAlias; readonly tokenAlias: bigint }
  | { readonly aliasType: TokenAliasType.UseValue; readonly tokenType: bigint; readonly tokenValue: Uint8Array }

/**
 * @public
 * The parameters of a request or response (moqt-22 §9.2). Each key is one parameter type;
 * the ones that may repeat are lists. Unknown and GREASE types are dropped when decoded.
 */
export interface MessageParameters {
  /** DELIVERY TIMEOUT for each Object, in ms. */
  objectDeliveryTimeout?: bigint
  authorizationToken?: AuthorizationToken[]
  /** How long (ms) a relay waits for a publisher of the track to appear. */
  rendezvousTimeout?: bigint
  /** DELIVERY TIMEOUT for each subgroup stream, in ms. */
  subgroupDeliveryTimeout?: bigint
  /** How long (ms) the subscription stays valid; 0 is no limit. */
  expires?: bigint
  /** The largest Object the publisher has, in SUBSCRIBE_OK and similar responses. */
  largestObject?: Location
  /** How long (ms) a fill may take. */
  fillTimeout?: bigint
  /** Whether Objects are forwarded to the subscriber. */
  forward?: boolean
  /** 0 is the highest priority, 255 the lowest. */
  subscriberPriority?: number
  locationFilter?: LocationFilter
  /** Only ASCENDING or DESCENDING travels on the wire; ORIGINAL is the absence of this parameter. */
  groupOrder?: GroupOrder
  /** The parameters of the fill fetch a SUBSCRIBE or REQUEST_UPDATE asks for (moqt-22 §3.4). */
  fillParameters?: MessageParameters
  switchFrom?: SwitchFrom
  subgroupFilter?: RangeFilter[]
  objectIdFilter?: RangeFilter[]
  priorityFilter?: RangeFilter[]
  objectPropertyFilter?: RangeFilter[]
  trackPropertyFilter?: RangeFilter[]
  /** Asks the publisher to start a new group at this group id or later. */
  newGroupRequest?: bigint
  trackNamespacePrefix?: Tuple
  /** Whether Objects carry their properties; absent means they do. */
  includeProperties?: boolean
  switchingSetAssignment?: SwitchingSetAssignment
}

/**
 * @public
 * The options of a SETUP message (moqt-22 §9.3).
 */
export interface SetupOptions {
  /** The MoQT path, for a raw QUIC session; WebTransport carries it in the URL. */
  path?: string
  authorizationToken?: AuthorizationToken[]
  maxAuthTokenCacheSize?: bigint
  /** Never sent over WebTransport, where the URL names the authority. */
  authority?: string
  /** How many Range Filter ranges the sender allows in one request (moqt-22 §9.3.2.6). */
  maxFilterRanges?: bigint
  moqtImplementation?: string
  maxRequestUpdates?: bigint
  /** The SSTS switching algorithms the sender supports (SSTS 4). */
  sstsAlgorithms?: bigint[]
  /** Options of types this table does not know, an extension's or GREASE, kept as raw pairs. */
  extensions?: KeyValuePair[]
}

/** @public Raised for a Range Filter the receiving side cannot apply, answered with INVALID_FILTER. */
export class InvalidFilterError extends MOQtailError {
  constructor(
    public readonly context: string,
    public readonly details: string,
  ) {
    super(`Invalid filter in ${context}: ${details}`)
  }
}

/** @public Every Range Filter parameter, with the type it travels as. */
export const RANGE_FILTERS: readonly (readonly [key: RangeFilterKey, type: number])[] = [
  ['subgroupFilter', 0x25],
  ['objectIdFilter', 0x26],
  ['priorityFilter', 0x27],
  ['objectPropertyFilter', 0x28],
  ['trackPropertyFilter', 0x29],
]

const MAX_U64 = 2n ** 64n - 1n
/** How many location fields each Location Filter Type carries, by type. */
export const FILTER_FIELDS = [0, 1, 2, 3, 4, 0]

/**
 * @public
 * A Location Filter, checked: each type carries exactly its own fields, and the end group
 * of a bounded one is not before its start group.
 */
export function locationFilter(
  filterType: LocationFilterType,
  startGroup?: bigint,
  startObject?: bigint,
  endGroup?: bigint,
  endObject?: bigint,
): LocationFilter {
  const count = FILTER_FIELDS[filterType]
  if (count === undefined) throw new ProtocolViolationError('LocationFilter', `unknown type ${filterType}`)

  const fields = [startGroup, startObject, endGroup, endObject]
  fields.forEach((field, i) => {
    if ((field !== undefined) !== i < count) {
      throw new ProtocolViolationError(
        'LocationFilter',
        `type ${filterType} carries ${count} fields; field ${i + 1} is ${field === undefined ? 'missing' : 'extra'}`,
      )
    }
  })
  if (endGroup !== undefined && endGroup < startGroup!) {
    throw new ProtocolViolationError('LocationFilter', `End Group ${endGroup} is before Start Group ${startGroup}`)
  }

  const filter: { -readonly [K in keyof LocationFilter]: LocationFilter[K] } = { filterType }
  if (startGroup !== undefined) filter.startGroup = startGroup
  if (startObject !== undefined) filter.startObject = startObject
  if (endGroup !== undefined) filter.endGroup = endGroup
  if (endObject !== undefined) filter.endObject = endObject

  return filter
}

/**
 * @public
 * Where a filter starts, given the track's Largest Object. A relative filter counts back
 * from it; with no Largest Object yet, the start is the first location.
 */
export function startLocation(filter: LocationFilter, largest?: Location): Location | undefined {
  switch (filter.filterType) {
    case LocationFilterType.None:
      return undefined
    case LocationFilterType.RelativeStart: {
      if (!largest) return new Location(0n, 0n)

      const group = largest.group + 1n - filter.startGroup!

      return new Location(group < 0n ? 0n : group > MAX_U64 ? MAX_U64 : group, 0n)
    }
    case LocationFilterType.NextObject:
      return largest ? new Location(largest.group, largest.object + 1n) : new Location(0n, 0n)
    default:
      return new Location(filter.startGroup!, filter.startObject!)
  }
}

/** @public The start of an absolute filter, or undefined for the others. */
export function absoluteStart(filter: LocationFilter): Location | undefined {
  const { filterType } = filter
  if (filterType < LocationFilterType.AbsoluteStart || filterType > LocationFilterType.AbsoluteRange) return undefined

  return new Location(filter.startGroup!, filter.startObject!)
}

/** @public The last location a bounded filter selects, or undefined when it has no end. */
export function endLocation(filter: LocationFilter): Location | undefined {
  if (filter.filterType === LocationFilterType.AbsoluteStartGroupEnd) return new Location(filter.endGroup!, MAX_U64)
  if (filter.filterType === LocationFilterType.AbsoluteRange) return new Location(filter.endGroup!, filter.endObject!)

  return undefined
}

/**
 * @public
 * The span a FETCH with this filter covers on a track whose Largest Object is `largest`,
 * or undefined when nothing has been published or the span is empty.
 */
export function fetchRange(filter: LocationFilter, largest?: Location): { start: Location; end: Location } | undefined {
  if (!largest) return undefined

  const start = startLocation(filter, largest) ?? new Location(0n, 0n)
  const filterEnd = endLocation(filter)
  const end = filterEnd && filterEnd.compare(largest) < 0 ? filterEnd : largest

  return start.compare(end) <= 0 ? { start, end } : undefined
}

/** @public The ranges a Range Filter codes, decoded. */
export function ranges(filter: RangeFilter): Range[] {
  const out: Range[] = []
  let priorEnd = 0n

  for (const [startDelta, endDelta] of filter.deltas) {
    const start = priorEnd + startDelta
    if (start > MAX_U64) throw new InvalidFilterError('RangeFilter.ranges', 'a range bound exceeds 2^64 - 1')
    if (endDelta === undefined) {
      out.push({ start })
      continue
    }

    const end = start + endDelta
    if (end > MAX_U64) throw new InvalidFilterError('RangeFilter.ranges', 'a range bound exceeds 2^64 - 1')

    out.push({ start, end })
    priorEnd = end
  }

  return out
}

/** @public The deltas that code `list`, which must be ascending with only its last range open-ended. */
export function encodeRanges(list: readonly Range[]): RangeDelta[] {
  const deltas: RangeDelta[] = []
  let priorEnd = 0n

  list.forEach((range, i) => {
    if (range.start < priorEnd || (range.end !== undefined && range.end < range.start)) {
      throw new InvalidFilterError('encodeRanges', 'ranges are not ascending')
    }

    if (range.end === undefined) {
      if (i + 1 !== list.length) throw new InvalidFilterError('encodeRanges', 'only the last range can be open-ended')

      deltas.push([range.start - priorEnd])

      return
    }

    deltas.push([range.start - priorEnd, range.end - range.start])
    priorEnd = range.end
  })

  return deltas
}

/**
 * The ranges of a filter the receiving side is asked to apply, checked: a property type
 * is even (it names a varint property), and a priority range stays within 0-255.
 */
export function validRanges(key: RangeFilterKey, filter: RangeFilter): Range[] {
  if (filter.propertyType !== undefined && filter.propertyType % 2n !== 0n) {
    throw new InvalidFilterError('RangeFilter', `property type 0x${filter.propertyType.toString(16)} is odd`)
  }

  const list = ranges(filter)
  if (key === 'priorityFilter') {
    const over = list.flatMap((r) => [r.start, r.end]).find((v) => v !== undefined && v > 255n)
    if (over !== undefined) throw new InvalidFilterError('RangeFilter', `priority ${over} is outside 0-255`)
  }

  return list
}

/** @public Whether `params`, or the fill they ask for, carry a Range Filter. */
export function carriesRangeFilter(params: MessageParameters): boolean {
  if (RANGE_FILTERS.some(([key]) => params[key]?.length)) return true

  return params.fillParameters ? carriesRangeFilter(params.fillParameters) : false
}

/**
 * Applies a REQUEST_UPDATE to the parameters of the request it updates (moqt-22 §10.9):
 * each parameter it carries replaces the current one of its type, and a Range Filter
 * removal drops the filters of its kind.
 */
export function applyParameterUpdate(current: MessageParameters, update: MessageParameters): MessageParameters {
  const next: MessageParameters = { ...current, ...update }

  for (const [key] of RANGE_FILTERS) {
    const filters = update[key]?.filter((f) => f.setId !== undefined)
    if (!filters) continue

    if (filters.length > 0) next[key] = filters
    else delete next[key]
  }

  return next
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest

  describe('location filters', () => {
    it('carry exactly the fields of their type', () => {
      expect(() => locationFilter(LocationFilterType.AbsoluteStart, 1n)).toThrow(ProtocolViolationError)
      expect(() => locationFilter(LocationFilterType.NextObject, 1n)).toThrow(ProtocolViolationError)
      expect(() => locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 5n, 0n, 4n)).toThrow(ProtocolViolationError)
    })

    it('span a fetch from the start to the end or the Largest Object, whichever comes first', () => {
      const largest = new Location(10n, 3n)

      expect(fetchRange(locationFilter(LocationFilterType.RelativeStart, 2n), largest)).toEqual({
        start: new Location(9n, 0n),
        end: largest,
      })

      const bounded = locationFilter(LocationFilterType.AbsoluteRange, 2n, 0n, 4n, 1n)

      expect(fetchRange(bounded, largest)?.end).toEqual(new Location(4n, 1n))
      expect(fetchRange(locationFilter(LocationFilterType.AbsoluteStart, 11n, 0n), largest)).toBeUndefined()
    })
  })

  describe('range filters', () => {
    it('decode the ranges their deltas code, and code ascending ranges only', () => {
      const deltas = encodeRanges([{ start: 2n, end: 4n }, { start: 7n }])

      expect(deltas).toEqual([[2n, 2n], [3n]])
      expect(ranges({ setId: 0, deltas })).toEqual([{ start: 2n, end: 4n }, { start: 7n }])
      expect(() => encodeRanges([{ start: 5n }, { start: 7n, end: 8n }])).toThrow(InvalidFilterError)
    })

    it('replace their kind on an update, and a removal drops it', () => {
      const current: MessageParameters = { forward: true, objectIdFilter: [{ setId: 0, deltas: [[1n]] }] }

      expect(applyParameterUpdate(current, { forward: false })).toEqual({ ...current, forward: false })
      expect(applyParameterUpdate(current, { objectIdFilter: [{ deltas: [] }] })).toEqual({ forward: true })
    })
  })
}
