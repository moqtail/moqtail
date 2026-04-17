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
 * Forwarding state for affected subscriptions.
 * Allowed wire values: 0 (don't forward) or 1 (forward).
 * Any other value is a PROTOCOL_VIOLATION.
 * Default (when omitted) is 1 (forward).
 */
export class Forward implements Parameter {
  static readonly TYPE = MessageParameterType.Forward

  constructor(public readonly forward: boolean) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(Forward.TYPE, this.forward ? 1n : 0n)
  }

  static fromKeyValuePair(pair: KeyValuePair): Forward | undefined {
    if (Number(pair.typeValue) !== Forward.TYPE || typeof pair.value !== 'bigint') return undefined
    if (pair.value !== 0n && pair.value !== 1n) {
      throw new ProtocolViolationError(
        'Forward.fromKeyValuePair',
        `FORWARD must be 0 or 1, got ${pair.value}`,
      )
    }
    return new Forward(pair.value === 1n)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('Forward', () => {
    test('roundtrips forward=true', () => {
      const orig = new Forward(true)
      const pair = orig.toKeyValuePair()
      const parsed = Forward.fromKeyValuePair(pair)
      expect(parsed?.forward).toBe(true)
    })
    test('roundtrips forward=false', () => {
      const orig = new Forward(false)
      const pair = orig.toKeyValuePair()
      const parsed = Forward.fromKeyValuePair(pair)
      expect(parsed?.forward).toBe(false)
    })
    test('fromKeyValuePair throws on invalid value', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.Forward, 2n)
      expect(() => Forward.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.DeliveryTimeout, 1n)
      expect(Forward.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
