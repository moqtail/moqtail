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

import { KeyValuePair } from '../common/pair'
import { AuthorizationToken } from './common'
import { DeliveryTimeout } from './message/delivery_timeout'
import { Expires } from './message/expires'
import { Forward } from './message/forward'
import { GroupOrderParam } from './message/group_order_param'
import { LargestObject } from './message/largest_object'
import { NewGroupRequest } from './message/new_group_request'
import { SubscriberPriority } from './message/subscriber_priority'
import { SubscriptionFilter } from './message/subscription_filter'

export type MessageParameter =
  | DeliveryTimeout
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
      DeliveryTimeout.fromKeyValuePair(pair) ??
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

  export function isDeliveryTimeout(param: MessageParameter): param is DeliveryTimeout {
    return param instanceof DeliveryTimeout
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

  export function isSubscriptionFilter(param: MessageParameter): param is SubscriptionFilter {
    return param instanceof SubscriptionFilter
  }

  export function isNewGroupRequest(param: MessageParameter): param is NewGroupRequest {
    return param instanceof NewGroupRequest
  }
}

/**
 * Builder for constructing a list of MessageParameters.
 * Mirrors the SetupParameters builder pattern.
 */
export class MessageParameters {
  private params: MessageParameter[] = []

  add(param: MessageParameter): this {
    this.params.push(param)
    return this
  }

  addDeliveryTimeout(timeout: bigint | number): this {
    return this.add(new DeliveryTimeout(BigInt(timeout)))
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
 * Applies a set of parameter updates to an existing parameter list.
 * For each update, replaces the matching parameter (by wire type value) or appends it.
 * Per spec: "If omitted from REQUEST_UPDATE/SUBSCRIBE_UPDATE, the value is unchanged."
 */
export function applyMessageParameterUpdate(
  current: MessageParameter[],
  updates: MessageParameter[],
): void {
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
  const { FilterType } = await import('../control/constant')
  const { Location } = await import('../common')

  describe('MessageParameter', () => {
    test('fromKeyValuePair returns undefined for unknown type', () => {
      const pair = KeyValuePair.tryNewVarInt(998n, 1n)
      expect(MessageParameter.fromKeyValuePair(pair)).toBeUndefined()
    })
  })

  describe('MessageParameters builder', () => {
    test('builds and roundtrips parameters', () => {
      const kvps = new MessageParameters()
        .addDeliveryTimeout(150n)
        .addForward(false)
        .addSubscriberPriority(42)
        .addSubscriptionFilter(new SubscriptionFilter(FilterType.AbsoluteRange, new Location(10n, 0n), 20n))
        .build()
        .map((p) => p.toKeyValuePair())

      const parsed = MessageParameters.fromKeyValuePairs(kvps)
      expect(parsed.length).toBe(4)
      expect(MessageParameter.isDeliveryTimeout(parsed[0]!) && parsed[0].timeout).toBe(150n)
      expect(MessageParameter.isForward(parsed[1]!) && parsed[1].forward).toBe(false)
      expect(MessageParameter.isSubscriberPriority(parsed[2]!) && parsed[2].priority).toBe(42)
      expect(
        MessageParameter.isSubscriptionFilter(parsed[3]!) && parsed[3].filterType,
      ).toBe(FilterType.AbsoluteRange)
    })

    test('fromKeyValuePairs skips unknown types', () => {
      const unknown = KeyValuePair.tryNewVarInt(998n, 1n)
      const valid = new DeliveryTimeout(100n).toKeyValuePair()
      const parsed = MessageParameters.fromKeyValuePairs([unknown, valid])
      expect(parsed.length).toBe(1)
      expect(MessageParameter.isDeliveryTimeout(parsed[0]!)).toBe(true)
    })
  })

  describe('applyMessageParameterUpdate', () => {
    test('replaces existing parameter and appends new ones', () => {
      const current: MessageParameter[] = [new SubscriberPriority(100), new Forward(true)]
      applyMessageParameterUpdate(current, [new SubscriberPriority(50), new DeliveryTimeout(500n)])
      expect(current.length).toBe(3)
      expect(current.some((p) => MessageParameter.isSubscriberPriority(p) && p.priority === 50)).toBe(true)
      expect(current.some((p) => MessageParameter.isForward(p) && p.forward === true)).toBe(true)
      expect(current.some((p) => MessageParameter.isDeliveryTimeout(p) && p.timeout === 500n)).toBe(true)
    })
  })
}
