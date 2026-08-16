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

import { KeyValuePair, deserializeKvpList, isBytes, isVarInt, serializeKvpList } from '../common/pair'
import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from '../common/byte_buffer'
import { greaseValue } from '../common/grease'
import { ProtocolViolationError } from '../error/error'
import { FilterType, GroupOrder } from '../control/constant'
import { Location } from '../common'
import { AuthorizationToken } from './common'
import { FillTimeout } from './message/fill_timeout'
import { ObjectDeliveryTimeout } from './message/object_delivery_timeout'
import { RendezvousTimeout } from './message/rendezvous_timeout'
import { SubgroupDeliveryTimeout } from './message/subgroup_delivery_timeout'
import { Expires } from './message/expires'
import { Forward } from './message/forward'
import { GroupOrderParam } from './message/group_order_param'
import { LargestObject } from './message/largest_object'
import { NewGroupRequest } from './message/new_group_request'
import { SubscriberPriority } from './message/subscriber_priority'
import { SubscriptionFilter } from './message/subscription_filter'

export type MessageParameter =
  | ObjectDeliveryTimeout
  | SubgroupDeliveryTimeout
  | RendezvousTimeout
  | FillTimeout
  | AuthorizationToken
  | Expires
  | LargestObject
  | Forward
  | SubscriberPriority
  | GroupOrderParam
  | SubscriptionFilter
  | NewGroupRequest

export namespace MessageParameter {
  /**
   * Parses a single KeyValuePair into a MessageParameter.
   * Returns undefined for unrecognized parameter types (be forgiving).
   * Still throws ProtocolViolationError for known types with invalid values.
   */
  export function fromKeyValuePair(pair: KeyValuePair): MessageParameter | undefined {
    return (
      ObjectDeliveryTimeout.fromKeyValuePair(pair) ??
      SubgroupDeliveryTimeout.fromKeyValuePair(pair) ??
      RendezvousTimeout.fromKeyValuePair(pair) ??
      FillTimeout.fromKeyValuePair(pair) ??
      AuthorizationToken.fromKeyValuePair(pair) ??
      Expires.fromKeyValuePair(pair) ??
      LargestObject.fromKeyValuePair(pair) ??
      Forward.fromKeyValuePair(pair) ??
      SubscriberPriority.fromKeyValuePair(pair) ??
      GroupOrderParam.fromKeyValuePair(pair) ??
      SubscriptionFilter.fromKeyValuePair(pair) ??
      NewGroupRequest.fromKeyValuePair(pair)
    )
  }

  export function toKeyValuePair(param: MessageParameter): KeyValuePair {
    return param.toKeyValuePair()
  }

  export function isObjectDeliveryTimeout(param: MessageParameter): param is ObjectDeliveryTimeout {
    return param instanceof ObjectDeliveryTimeout
  }

  export function isSubgroupDeliveryTimeout(param: MessageParameter): param is SubgroupDeliveryTimeout {
    return param instanceof SubgroupDeliveryTimeout
  }

  export function isRendezvousTimeout(param: MessageParameter): param is RendezvousTimeout {
    return param instanceof RendezvousTimeout
  }

  export function isFillTimeout(param: MessageParameter): param is FillTimeout {
    return param instanceof FillTimeout
  }

  export function isAuthorizationToken(param: MessageParameter): param is AuthorizationToken {
    return param instanceof AuthorizationToken
  }

  export function isExpires(param: MessageParameter): param is Expires {
    return param instanceof Expires
  }

  export function isLargestObject(param: MessageParameter): param is LargestObject {
    return param instanceof LargestObject
  }

  export function isForward(param: MessageParameter): param is Forward {
    return param instanceof Forward
  }

  export function isSubscriberPriority(param: MessageParameter): param is SubscriberPriority {
    return param instanceof SubscriberPriority
  }

  export function isGroupOrderParam(param: MessageParameter): param is GroupOrderParam {
    return param instanceof GroupOrderParam
  }

  /** The negotiated Group Order, or {@link (GroupOrder:enum).Original} when unparameterized. */
  export function groupOrderOf(params: readonly MessageParameter[]): GroupOrder {
    return params.find(isGroupOrderParam)?.order ?? GroupOrder.Original
  }

  export function isSubscriptionFilter(param: MessageParameter): param is SubscriptionFilter {
    return param instanceof SubscriptionFilter
  }

  export function isNewGroupRequest(param: MessageParameter): param is NewGroupRequest {
    return param instanceof NewGroupRequest
  }
}

/**
 * Builder for constructing a list of MessageParameters.
 * Mirrors the SetupOptions builder pattern.
 */
export class MessageParameters {
  private params: MessageParameter[] = []

  add(param: MessageParameter): this {
    this.params.push(param)
    return this
  }

  addObjectDeliveryTimeout(timeout: bigint | number): this {
    return this.add(new ObjectDeliveryTimeout(BigInt(timeout)))
  }

  addSubgroupDeliveryTimeout(timeout: bigint | number): this {
    return this.add(new SubgroupDeliveryTimeout(BigInt(timeout)))
  }

  addRendezvousTimeout(timeout: bigint | number): this {
    return this.add(new RendezvousTimeout(BigInt(timeout)))
  }

  addFillTimeout(timeout: bigint | number): this {
    return this.add(new FillTimeout(BigInt(timeout)))
  }

  addAuthorizationToken(token: AuthorizationToken): this {
    return this.add(token)
  }

  addExpires(expires: bigint | number): this {
    return this.add(new Expires(BigInt(expires)))
  }

  addForward(forward: boolean): this {
    return this.add(new Forward(forward))
  }

  addSubscriberPriority(priority: number): this {
    return this.add(new SubscriberPriority(priority))
  }

  addGroupOrder(order: GroupOrderParam['order']): this {
    return this.add(new GroupOrderParam(order))
  }

  addSubscriptionFilter(filter: SubscriptionFilter): this {
    return this.add(filter)
  }

  addNewGroupRequest(group: bigint | number): this {
    return this.add(new NewGroupRequest(BigInt(group)))
  }

  build(): MessageParameter[] {
    return [...this.params]
  }

  /**
   * Parses an array of KeyValuePairs into a list of MessageParameters.
   * Unrecognized parameter types are silently skipped.
   * Known parameter types with invalid values still throw ProtocolViolationError.
   */
  static fromKeyValuePairs(pairs: KeyValuePair[]): MessageParameter[] {
    const result: MessageParameter[] = []
    for (const pair of pairs) {
      const parsed = MessageParameter.fromKeyValuePair(pair)
      if (parsed !== undefined) result.push(parsed)
    }
    return result
  }
}

/**
 * LARGEST_OBJECT (0x09) is a bare Location -- two consecutive varints with no
 * length prefix. Its Type is odd, so the generic KVP parity rule would read a
 * length prefix and desync. This is a message-parameter encoding: the same Type
 * number in the setup-option namespace means something else, so the rule lives
 * here rather than in the shared codec.
 */
function isLocationMessageParam(typeValue: bigint): boolean {
  return typeValue === 0x09n
}

/**
 * FORWARD (0x10), SUBSCRIBER_PRIORITY (0x20) and GROUP_ORDER (0x22) carry a
 * single uint8, not the generic even-Type varint. These Types are even, so
 * without this the parity rule would read a varint and desync on any value
 * >= 64 (e.g. the default SUBSCRIBER_PRIORITY of 128 = 0x80).
 */
function isUint8MessageParam(typeValue: bigint): boolean {
  return typeValue === 0x10n || typeValue === 0x20n || typeValue === 0x22n
}

/**
 * Serializes a list of message-parameter KVPs, delta-encoding the Types.
 * Mirrors {@link serializeKvpList} but honours per-parameter value encodings.
 */
export function serializeMessageParameterKvps(items: KeyValuePair[]): FrozenByteBuffer {
  const sorted = [...items].sort((a, b) => (a.typeValue < b.typeValue ? -1 : a.typeValue > b.typeValue ? 1 : 0))
  const buf = new ByteBuffer()
  let prevType = 0n
  for (const kvp of sorted) {
    buf.putVI(kvp.typeValue - prevType)
    if (isVarInt(kvp) && isUint8MessageParam(kvp.typeValue)) {
      if (kvp.value < 0n || kvp.value > 255n) {
        throw new ProtocolViolationError(
          'serializeMessageParameterKvps',
          `uint8 parameter 0x${kvp.typeValue.toString(16)} value ${kvp.value} exceeds 255`,
        )
      }
      buf.putU8(Number(kvp.value))
    } else if (isVarInt(kvp)) {
      buf.putVI(kvp.value)
    } else if (isBytes(kvp) && isLocationMessageParam(kvp.typeValue)) {
      buf.putBytes(kvp.value)
    } else if (isBytes(kvp)) {
      buf.putLengthPrefixedBytes(kvp.value)
    }
    prevType = kvp.typeValue
  }
  return buf.freeze()
}

/**
 * Reads exactly `count` delta-encoded message-parameter KVPs.
 * Mirrors {@link deserializeKvpList} but honours per-parameter value encodings.
 */
export function deserializeMessageParameterKvps(buf: BaseByteBuffer, count: number | bigint): KeyValuePair[] {
  const n = typeof count === 'bigint' ? Number(count) : count
  const items: KeyValuePair[] = new Array(n)
  let prevType = 0n
  for (let i = 0; i < n; i++) {
    const typeValue = prevType + buf.getVI()
    if (isUint8MessageParam(typeValue)) {
      items[i] = KeyValuePair.tryNewVarInt(typeValue, BigInt(buf.getU8()))
    } else if (isLocationMessageParam(typeValue)) {
      const loc = new ByteBuffer()
      loc.putVI(buf.getVI())
      loc.putVI(buf.getVI())
      items[i] = KeyValuePair.tryNewBytes(typeValue, loc.toUint8Array())
    } else {
      items[i] = KeyValuePair.deserializeValue(buf, typeValue)
    }
    prevType = typeValue
  }
  return items
}

/**
 * Applies a set of parameter updates to an existing parameter list.
 * For each update, replaces the matching parameter (by wire type value) or appends it.
 * Per spec: "If omitted from REQUEST_UPDATE/SUBSCRIBE_UPDATE, the value is unchanged."
 */
export function applyMessageParameterUpdate(current: MessageParameter[], updates: MessageParameter[]): void {
  for (const update of updates) {
    const updateType = update.toKeyValuePair().typeValue
    const idx = current.findIndex((p) => p.toKeyValuePair().typeValue === updateType)
    if (idx >= 0) {
      current[idx] = update
    } else {
      current.push(update)
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MessageParameter', () => {
    test('fromKeyValuePair returns undefined for unknown type', () => {
      const pair = KeyValuePair.tryNewVarInt(998n, 1n)
      expect(MessageParameter.fromKeyValuePair(pair)).toBeUndefined()
    })
  })

  describe('message parameter wire encodings', () => {
    test('LARGEST_OBJECT is an unprefixed Location', () => {
      // Type Delta 0x09 then the Group and Object varints, no length prefix.
      // These are the bytes another implementation put on the wire for {1, 13}.
      const wire = new Uint8Array([0x09, 0x01, 0x0d])

      const buf = new ByteBuffer()
      buf.putBytes(wire)
      const frozen = buf.freeze()
      const params = MessageParameters.fromKeyValuePairs(deserializeMessageParameterKvps(frozen, 1))

      expect(params).toEqual([new LargestObject(new Location(1n, 13n))])
      expect(frozen.remaining).toBe(0)
      expect(serializeMessageParameterKvps(params.map((p) => p.toKeyValuePair())).toUint8Array()).toEqual(wire)
    })

    test('SUBSCRIBER_PRIORITY is a single byte, not a varint', () => {
      // 128 is the default priority and the first value where uint8 and varint
      // encodings differ: one byte 0x80 rather than the two bytes 0x40 0x80.
      const params = [new SubscriberPriority(128)]
      const wire = serializeMessageParameterKvps(params.map((p) => p.toKeyValuePair())).toUint8Array()

      expect(wire).toEqual(new Uint8Array([0x20, 0x80]))

      const buf = new ByteBuffer()
      buf.putBytes(wire)
      const frozen = buf.freeze()
      expect(MessageParameters.fromKeyValuePairs(deserializeMessageParameterKvps(frozen, 1))).toEqual(params)
    })
  })

  describe('MessageParameters builder', () => {
    test('builds and roundtrips parameters', () => {
      const kvps = new MessageParameters()
        .addObjectDeliveryTimeout(150n)
        .addForward(false)
        .addSubscriberPriority(42)
        .addSubscriptionFilter(new SubscriptionFilter(FilterType.AbsoluteRange, new Location(10n, 0n), 20n))
        .build()
        .map((p) => p.toKeyValuePair())

      const parsed = MessageParameters.fromKeyValuePairs(kvps)
      expect(parsed.length).toBe(4)
      expect(MessageParameter.isObjectDeliveryTimeout(parsed[0]!) && parsed[0].timeout).toBe(150n)
      expect(MessageParameter.isForward(parsed[1]!) && parsed[1].forward).toBe(false)
      expect(MessageParameter.isSubscriberPriority(parsed[2]!) && parsed[2].priority).toBe(42)
      expect(MessageParameter.isSubscriptionFilter(parsed[3]!) && parsed[3].filterType).toBe(FilterType.AbsoluteRange)
    })

    // §14: a greased parameter is just another unknown one -- skipped, never fatal.
    test('fromKeyValuePairs skips a greased type', () => {
      const grease = KeyValuePair.tryNewVarInt(greaseValue(1)!, 1n)
      const valid = new ObjectDeliveryTimeout(100n).toKeyValuePair()
      expect(MessageParameters.fromKeyValuePairs([grease, valid])).toEqual([new ObjectDeliveryTimeout(100n)])
    })

    test('fromKeyValuePairs skips unknown types', () => {
      const unknown = KeyValuePair.tryNewVarInt(998n, 1n)
      const valid = new ObjectDeliveryTimeout(100n).toKeyValuePair()
      const parsed = MessageParameters.fromKeyValuePairs([unknown, valid])
      expect(parsed.length).toBe(1)
      expect(MessageParameter.isObjectDeliveryTimeout(parsed[0]!)).toBe(true)
    })
  })

  describe('applyMessageParameterUpdate', () => {
    test('replaces existing parameter and appends new ones', () => {
      const current: MessageParameter[] = [new SubscriberPriority(100), new Forward(true)]
      applyMessageParameterUpdate(current, [new SubscriberPriority(50), new ObjectDeliveryTimeout(500n)])
      expect(current.length).toBe(3)
      expect(current.some((p) => MessageParameter.isSubscriberPriority(p) && p.priority === 50)).toBe(true)
      expect(current.some((p) => MessageParameter.isForward(p) && p.forward === true)).toBe(true)
      expect(current.some((p) => MessageParameter.isObjectDeliveryTimeout(p) && p.timeout === 500n)).toBe(true)
    })
  })

  describe('delta-encoding regression', () => {
    test('bug report wire format is delta-encoded', () => {
      // Regression for the reported interop bug: SUBSCRIBER_PRIORITY (0x20),
      // FORWARD (0x10) and SUBSCRIPTION_FILTER (0x21), built in non-ascending
      // insertion order. A spec-compliant v16 peer decodes Type as a delta
      // from the previous Type in the list; encoding them "as-is" (absolute)
      // made a correct delta-decoder compute the wrong types.
      const params: MessageParameter[] = [
        new SubscriberPriority(0),
        new Forward(true),
        new SubscriptionFilter(FilterType.LatestObject),
      ]
      const frozen = serializeKvpList(params.map((p) => p.toKeyValuePair()))

      // Decode independently of deserializeKvpList, using raw delta
      // semantics, to prove the wire bytes are genuinely delta-encoded and
      // not just self-consistent with our own (potentially still-buggy) decoder.
      let prevType = 0n
      const types: bigint[] = []
      while (frozen.remaining > 0) {
        const kvp = KeyValuePair.deserializeDelta(frozen, prevType)
        prevType = kvp.typeValue
        types.push(prevType)
      }
      expect(types).toEqual([Forward.TYPE, SubscriberPriority.TYPE, SubscriptionFilter.TYPE].map(BigInt))
    })

    test('decodes a delta-encoded peer stream', () => {
      // Simulates a spec-compliant peer sending FORWARD then SUBSCRIBER_PRIORITY,
      // correctly delta-encoded. This is the direction the reporter's own
      // workaround (disabling delta decoding) broke.
      const buf = new ByteBuffer()
      buf.putVI(BigInt(Forward.TYPE)) // delta from 0 -> Forward
      buf.putVI(1n) // true
      buf.putVI(BigInt(SubscriberPriority.TYPE) - BigInt(Forward.TYPE)) // delta from Forward -> SubscriberPriority
      buf.putVI(5n)
      const frozen = buf.freeze()

      const kvps = deserializeKvpList(frozen, 2)
      const params = MessageParameters.fromKeyValuePairs(kvps)
      expect(params).toEqual([new Forward(true), new SubscriberPriority(5)])
    })
  })
}
