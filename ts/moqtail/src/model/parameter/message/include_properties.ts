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
 * Whether the response carries Track Properties: the OK answering a SUBSCRIBE,
 * TRACK_STATUS or FETCH, or each PUBLISH a SUBSCRIBE_TRACKS brings. With 0 the
 * Track Properties field is still there, but empty.
 * Allowed wire values: 0 or 1; any other value is a PROTOCOL_VIOLATION.
 * Default (when omitted) is 1 (include them).
 */
export class IncludeProperties implements Parameter {
  static readonly TYPE = MessageParameterType.IncludeProperties

  constructor(public readonly include: boolean) {}

  toKeyValuePair(): KeyValuePair {
    // A uint8 on an odd Type: its value shape, not its parity, decides the encoding.
    return KeyValuePair.newVarInt(IncludeProperties.TYPE, this.include ? 1n : 0n)
  }

  static fromKeyValuePair(pair: KeyValuePair): IncludeProperties | undefined {
    if (Number(pair.typeValue) !== IncludeProperties.TYPE || typeof pair.value !== 'bigint') return undefined
    if (pair.value !== 0n && pair.value !== 1n) {
      throw new ProtocolViolationError(
        'IncludeProperties.fromKeyValuePair',
        `INCLUDE_PROPERTIES must be 0 or 1, got ${pair.value}`,
      )
    }
    return new IncludeProperties(pair.value === 1n)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('IncludeProperties', () => {
    test('roundtrips both values', () => {
      for (const include of [false, true]) {
        const parsed = IncludeProperties.fromKeyValuePair(new IncludeProperties(include).toKeyValuePair())
        expect(parsed?.include).toBe(include)
      }
    })
    test('fromKeyValuePair throws on a value other than 0 or 1', () => {
      const pair = KeyValuePair.newVarInt(MessageParameterType.IncludeProperties, 2n)
      expect(() => IncludeProperties.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.Forward, 0n)
      expect(IncludeProperties.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
