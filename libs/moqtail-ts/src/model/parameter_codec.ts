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

/** The codecs of message parameters and SETUP options (moqt-22 §9), one table row per type. */

import { byte, flag, kvps, location, text, tuple, varint, wrapped } from './codec'
import type { Codec, Entry } from './codec'
import { GroupOrder, SwitchMode, TokenAliasType } from './constants'
import { ProtocolViolationError } from './error/error'
import { FILTER_FIELDS, locationFilter } from './parameters'
import type {
  AuthorizationToken,
  LocationFilter,
  MessageParameters,
  RangeDelta,
  RangeFilter,
  SetupOptions,
  SwitchFrom,
  SwitchingSetAssignment,
} from './parameters'

const MAX_U64 = 2n ** 64n - 1n
/** The parameters allowed inside FILL_PARAMETERS (moqt-22 §9.2.2.9). */
const FILL_ALLOWED = [0x0a, 0x20, 0x21, 0x22, 0x25, 0x26, 0x27, 0x28]

const groupOrder: Codec<GroupOrder> = [
  (buf) => {
    const order = buf.getU8()
    if (order !== GroupOrder.Ascending && order !== GroupOrder.Descending) {
      throw new ProtocolViolationError('GroupOrder', `GROUP_ORDER must be 1 or 2, got ${order}`)
    }

    return order
  },
  (buf, order) => buf.putU8(order),
]

/** A Location Filter value, as a parameter carries it and TRACK_STATUS inlines it. */
export const locationFilterValue: Codec<LocationFilter> = [
  (buf) => {
    const filterType = buf.getNumberVI()
    const count = FILTER_FIELDS[filterType]
    if (count === undefined) throw new ProtocolViolationError('LocationFilter', `unknown type ${filterType}`)

    const startGroup = count > 0 ? buf.getVI() : undefined
    const startObject = count > 1 ? buf.getVI() : undefined
    const endGroup = count > 2 ? startGroup! + buf.getVI() : undefined
    if (endGroup !== undefined && endGroup > MAX_U64) {
      throw new ProtocolViolationError('LocationFilter', 'Start Group plus End Group Delta exceeds 2^64 - 1')
    }

    const endObject = count > 3 ? buf.getVI() : undefined

    return locationFilter(filterType, startGroup, startObject, endGroup, endObject)
  },
  (buf, filter) => {
    const count = FILTER_FIELDS[filter.filterType]!

    buf.putVI(filter.filterType)
    if (count > 0) buf.putVI(filter.startGroup!)
    if (count > 1) buf.putVI(filter.startObject!)
    if (count > 2) buf.putVI(filter.endGroup! - filter.startGroup!)
    if (count > 3) buf.putVI(filter.endObject!)
  },
]

function rangeFilter(hasPropertyType: boolean): Codec<RangeFilter> {
  return wrapped<RangeFilter>([
    (buf) => {
      if (buf.remaining === 0) return { deltas: [] }

      const setId = buf.getU8()
      const propertyType = hasPropertyType ? buf.getVI() : undefined
      const deltas: RangeDelta[] = []

      while (buf.remaining > 0) {
        const start = buf.getVI()

        deltas.push(buf.remaining > 0 ? [start, buf.getVI()] : [start])
      }

      return propertyType === undefined ? { setId, deltas } : { setId, propertyType, deltas }
    },
    (buf, filter) => {
      if (filter.setId === undefined) return
      if (hasPropertyType !== (filter.propertyType !== undefined)) {
        throw new ProtocolViolationError('RangeFilter', 'wrong Property Type presence')
      }

      buf.putU8(filter.setId)
      if (filter.propertyType !== undefined) buf.putVI(filter.propertyType)

      filter.deltas.forEach(([start, end], i) => {
        if (end === undefined && i + 1 < filter.deltas.length) {
          throw new ProtocolViolationError('RangeFilter', 'only the last range can be open-ended')
        }

        buf.putVI(start)
        if (end !== undefined) buf.putVI(end)
      })
    },
  ])
}

const switchFrom: Codec<SwitchFrom> = wrapped([
  (buf) => {
    const requestId = buf.getVI()
    const mode = buf.getNumberVI()
    if (mode !== SwitchMode.Hard && mode !== SwitchMode.Soft) {
      throw new ProtocolViolationError('SwitchFrom', `invalid mode ${mode}`)
    }

    const flags = buf.getU8()
    if (flags & 0x7f) throw new ProtocolViolationError('SwitchFrom', `reserved bits must be zero, got ${flags}`)

    return { requestId, mode, publishDone: flags === 0x80 }
  },
  (buf, v) => {
    buf.putVI(v.requestId)
    buf.putVI(v.mode)
    buf.putU8(v.publishDone ? 0x80 : 0)
  },
])

const switchingSetAssignment: Codec<SwitchingSetAssignment> = wrapped([
  (buf): SwitchingSetAssignment => {
    const switchingSetId = buf.getVI()
    const algorithmId = buf.getVI()
    const throughputThresholdKbps = buf.getVI()
    const setThroughputWeight = buf.getVI()
    if (setThroughputWeight < 1n || setThroughputWeight > 10n) {
      throw new ProtocolViolationError(
        'SwitchingSetAssignment',
        `SET THROUGHPUT WEIGHT must be 1-10, got ${setThroughputWeight}`,
      )
    }

    const activateSwitching = buf.getVI()
    const setRank = buf.remaining > 0 ? buf.getU8() : 0

    // Algorithm 0 is fixed at the draft's six fields; a private knob wearing its
    // id would be read as the next standard field by a conformant peer.
    if (algorithmId === 0n) {
      if (buf.remaining > 0) {
        throw new ProtocolViolationError('SwitchingSetAssignment', 'excess bytes in SWITCHING_SET_ASSIGNMENT parameter')
      }

      return { switchingSetId, algorithmId, throughputThresholdKbps, setThroughputWeight, activateSwitching, setRank }
    }

    // The extension is the latency budget followed by the algorithm's own
    // varints. A peer that predates it sends nothing here, which reads as no
    // budget rather than a malformed assignment.
    const subscriberBufferMs = buf.remaining > 0 ? buf.getVI() : undefined
    const algorithmParams: bigint[] = []
    while (buf.remaining > 0) algorithmParams.push(buf.getVI())

    return {
      switchingSetId,
      algorithmId,
      throughputThresholdKbps,
      setThroughputWeight,
      activateSwitching,
      setRank,
      ...(subscriberBufferMs !== undefined && { subscriberBufferMs }),
      ...(algorithmParams.length > 0 && { algorithmParams }),
    }
  },
  (buf, v) => {
    buf.putVI(v.switchingSetId)
    buf.putVI(v.algorithmId)
    buf.putVI(v.throughputThresholdKbps)
    buf.putVI(v.setThroughputWeight)
    buf.putVI(v.activateSwitching)
    buf.putU8(v.setRank)

    // Algorithm 0 stays byte-identical to the draft, even if the caller
    // attached extension fields.
    if (v.algorithmId === 0n) return
    if (v.subscriberBufferMs === undefined && (v.algorithmParams?.length ?? 0) === 0) return

    buf.putVI(v.subscriberBufferMs ?? 0n)
    for (const param of v.algorithmParams ?? []) buf.putVI(param)
  },
])

const authorizationToken: Codec<AuthorizationToken> = wrapped<AuthorizationToken>([
  (buf) => {
    const aliasType = buf.getNumberVI()
    if (aliasType > TokenAliasType.UseValue) {
      throw new ProtocolViolationError('AuthorizationToken', `unknown alias type ${aliasType}`)
    }

    const tokenAlias = aliasType === TokenAliasType.UseValue ? undefined : buf.getVI()

    if (aliasType === TokenAliasType.Delete || aliasType === TokenAliasType.UseAlias) {
      return { aliasType, tokenAlias: tokenAlias! }
    }

    const tokenType = buf.getVI()
    const tokenValue = buf.getBytes(buf.remaining)
    if (aliasType === TokenAliasType.Register) return { aliasType, tokenAlias: tokenAlias!, tokenType, tokenValue }

    return { aliasType, tokenType, tokenValue }
  },
  (buf, token) => {
    buf.putVI(token.aliasType)

    switch (token.aliasType) {
      case TokenAliasType.Delete:
      case TokenAliasType.UseAlias:
        buf.putVI(token.tokenAlias)
        break
      case TokenAliasType.Register:
        buf.putVI(token.tokenAlias)
        buf.putVI(token.tokenType)
        buf.putBytes(token.tokenValue)
        break
      case TokenAliasType.UseValue:
        buf.putVI(token.tokenType)
        buf.putBytes(token.tokenValue)
        break
    }
  },
])

const varints: Codec<bigint[]> = wrapped([
  (buf) => {
    const values: bigint[] = []

    while (buf.remaining > 0) values.push(buf.getVI())

    return values
  },
  (buf, values) => {
    for (const value of values) buf.putVI(value)
  },
])

// Sorted by type: the list is delta-coded in this order (moqt-22 §1.4.2)
const PARAMETERS: Entry[] = [
  [0x02, 'objectDeliveryTimeout', varint],
  [0x03, 'authorizationToken', authorizationToken, 'repeated'],
  [0x04, 'rendezvousTimeout', varint],
  [0x06, 'subgroupDeliveryTimeout', varint],
  [0x08, 'expires', varint],
  [0x09, 'largestObject', location],
  [0x0a, 'fillTimeout', varint],
  [0x10, 'forward', flag],
  [0x20, 'subscriberPriority', byte],
  [0x21, 'locationFilter', locationFilterValue],
  [0x22, 'groupOrder', groupOrder],
  [0x24, 'switchFrom', switchFrom],
  [0x25, 'subgroupFilter', rangeFilter(false), 'repeated'],
  [0x26, 'objectIdFilter', rangeFilter(false), 'repeated'],
  [0x27, 'priorityFilter', rangeFilter(false), 'repeated'],
  [0x28, 'objectPropertyFilter', rangeFilter(true), 'repeated'],
  [0x29, 'trackPropertyFilter', rangeFilter(true), 'repeated'],
  [0x32, 'newGroupRequest', varint],
  [0x34, 'trackNamespacePrefix', tuple],
  [0x35, 'includeProperties', flag],
  [0x41, 'switchingSetAssignment', switchingSetAssignment],
]
PARAMETERS.splice(11, 0, [
  0x23,
  'fillParameters',
  wrapped(kvps<MessageParameters>(PARAMETERS, 'counted', { allowed: FILL_ALLOWED })),
])

/** A count-prefixed parameter list. */
export const parameters: Codec<MessageParameters> = kvps<MessageParameters>(PARAMETERS, 'counted')

/** The options of a SETUP message, which run to the end of its payload. */
export const setupOptions: Codec<SetupOptions> = kvps<SetupOptions>(
  [
    [0x01, 'path', text],
    [0x03, 'authorizationToken', authorizationToken, 'repeated'],
    [0x04, 'maxAuthTokenCacheSize', varint],
    [0x05, 'authority', text],
    [0x06, 'maxFilterRanges', varint],
    [0x07, 'moqtImplementation', text],
    [0x08, 'maxRequestUpdates', varint],
    [0x09, 'sstsAlgorithms', varints],
  ],
  'toEnd',
  { extensions: 'extensions' },
)

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
  const { ByteBuffer, FrozenByteBuffer } = await import('./common/byte_buffer')
  const { greaseValue } = await import('./common/grease')
  const { GroupOrder } = await import('./constants')
  type MessageParameters = import('./parameters').MessageParameters

  const encode = (params: MessageParameters) => {
    const buf = new ByteBuffer()
    parameters[1](buf, params)

    return buf.toUint8Array()
  }

  const decode = (bytes: Uint8Array) => parameters[0](new FrozenByteBuffer(bytes))

  describe('parameters', () => {
    it('skips unknown and GREASE types by their parity', () => {
      const buf = new ByteBuffer()
      buf.putVI(3)
      buf.putVI(0x02) // DELIVERY TIMEOUT
      buf.putVI(100)
      buf.putVI(greaseValue(1)! - 0x02n) // an even GREASE type carries a varint
      buf.putVI(7)
      buf.putVI(1) // an odd unknown type carries bytes
      buf.putLengthPrefixedBytes(new Uint8Array([9, 9]))
      expect(decode(buf.toUint8Array())).toEqual({ objectDeliveryTimeout: 100n })
    })

    it('allows only the fill parameters inside FILL_PARAMETERS', () => {
      expect(decode(encode({ fillParameters: { fillTimeout: 5n } }))).toEqual({ fillParameters: { fillTimeout: 5n } })
      expect(() => encode({ fillParameters: { forward: true } })).toThrow(ProtocolViolationError)
    })

    it('refuses a FORWARD other than 0 or 1, and a GROUP_ORDER of ORIGINAL', () => {
      const forward = encode({ forward: true })

      forward[forward.length - 1] = 2
      expect(() => decode(forward)).toThrow(ProtocolViolationError)
      expect(() => decode(encode({ groupOrder: GroupOrder.Original }))).toThrow(ProtocolViolationError)
    })

    it('carries a Range Filter removal as an empty value', () => {
      expect(decode(encode({ priorityFilter: [{ deltas: [] }] }))).toEqual({ priorityFilter: [{ deltas: [] }] })
    })
  })

  describe('SWITCHING_SET_ASSIGNMENT', () => {
    const base = {
      switchingSetId: 7n,
      algorithmId: 0xff04n,
      throughputThresholdKbps: 2500n,
      setThroughputWeight: 3n,
      activateSwitching: 2n,
      setRank: 1,
    }

    const parameter = (value: Uint8Array) => {
      const buf = new ByteBuffer()
      buf.putVI(1)
      buf.putVI(0x41)
      buf.putLengthPrefixedBytes(value)

      return buf.toUint8Array()
    }

    it('writes the draft fields, then the budget and the algorithm varints, in order', () => {
      const assignment = { ...base, subscriberBufferMs: 250n, algorithmParams: [25n, 2n, 0n] }
      const value = new ByteBuffer()
      value.putVI(7n)
      value.putVI(0xff04n)
      value.putVI(2500n)
      value.putVI(3n)
      value.putVI(2n)
      value.putU8(1)
      value.putVI(250n)
      value.putVI(25n)
      value.putVI(2n)
      value.putVI(0n)

      expect(encode({ switchingSetAssignment: assignment })).toEqual(parameter(value.toUint8Array()))
      expect(decode(encode({ switchingSetAssignment: assignment }))).toEqual({ switchingSetAssignment: assignment })
    })

    it('reads a non-default assignment that predates the extension', () => {
      expect(decode(encode({ switchingSetAssignment: base }))).toEqual({ switchingSetAssignment: base })
    })

    it('keeps algorithm 0 byte-identical and ignores extension fields', () => {
      const six = { ...base, algorithmId: 0n }
      const asked = { ...six, subscriberBufferMs: 250n, algorithmParams: [25n] }

      expect(encode({ switchingSetAssignment: asked })).toEqual(encode({ switchingSetAssignment: six }))
      expect(decode(encode({ switchingSetAssignment: asked }))).toEqual({ switchingSetAssignment: six })
    })

    it('rejects extension bytes under algorithm 0', () => {
      const value = new ByteBuffer()
      value.putVI(7n)
      value.putVI(0n)
      value.putVI(2500n)
      value.putVI(3n)
      value.putVI(2n)
      value.putU8(1)
      value.putVI(250n)

      expect(() => decode(parameter(value.toUint8Array()))).toThrow(ProtocolViolationError)
    })
  })
}
