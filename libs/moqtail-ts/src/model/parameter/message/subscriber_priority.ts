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

import { KeyValuePair } from '../../common/pair'
import { ProtocolViolationError } from '../../error/error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'

/**
 * Priority of a subscription relative to others in the same session.
 * Lower numbers have higher priority. Range: 0-255.
 * Values outside this range are a PROTOCOL_VIOLATION.
 * Default (when omitted from SUBSCRIBE, PUBLISH_OK, or FETCH) is 128.
 */
export class SubscriberPriority implements Parameter {
  static readonly TYPE = MessageParameterType.SubscriberPriority

  constructor(public readonly priority: number) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(SubscriberPriority.TYPE, BigInt(this.priority))
  }

  static fromKeyValuePair(pair: KeyValuePair): SubscriberPriority | undefined {
    if (Number(pair.typeValue) !== SubscriberPriority.TYPE || typeof pair.value !== 'bigint') return undefined
    if (pair.value < 0n || pair.value > 255n) {
      throw new ProtocolViolationError(
        'SubscriberPriority.fromKeyValuePair',
        `SUBSCRIBER_PRIORITY must be 0-255, got ${pair.value}`,
      )
    }
    return new SubscriberPriority(Number(pair.value))
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SubscriberPriority', () => {
    test('roundtrips correctly', () => {
      const orig = new SubscriberPriority(42)
      const pair = orig.toKeyValuePair()
      const parsed = SubscriberPriority.fromKeyValuePair(pair)
      expect(parsed?.priority).toBe(42)
    })
    test('accepts boundary values 0 and 255', () => {
      expect(SubscriberPriority.fromKeyValuePair(new SubscriberPriority(0).toKeyValuePair())?.priority).toBe(0)
      expect(SubscriberPriority.fromKeyValuePair(new SubscriberPriority(255).toKeyValuePair())?.priority).toBe(255)
    })
    test('fromKeyValuePair throws on value > 255', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.SubscriberPriority, 256n)
      expect(() => SubscriberPriority.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.DeliveryTimeout, 100n)
      expect(SubscriberPriority.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
