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
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'

/**
 * Duration in milliseconds a relay SHOULD wait for a publisher of a track that has none
 * before answering the SUBSCRIBE. SUBSCRIBE only (§10.2.6).
 */
export class RendezvousTimeout implements Parameter {
  static readonly TYPE = MessageParameterType.RendezvousTimeout

  constructor(public readonly timeout: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(RendezvousTimeout.TYPE, this.timeout)
  }

  static fromKeyValuePair(pair: KeyValuePair): RendezvousTimeout | undefined {
    if (Number(pair.typeValue) !== RendezvousTimeout.TYPE || typeof pair.value !== 'bigint') return undefined
    return new RendezvousTimeout(pair.value)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('RendezvousTimeout', () => {
    test('roundtrips correctly', () => {
      const pair = new RendezvousTimeout(1500n).toKeyValuePair()
      expect(RendezvousTimeout.fromKeyValuePair(pair)?.timeout).toBe(1500n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.ObjectDeliveryTimeout, 100n)
      expect(RendezvousTimeout.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
