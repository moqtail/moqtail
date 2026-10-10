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
import { KeyValueFormattingError, ProtocolViolationError } from '../../error/error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'

/**
 * Whether the OK sent in response carries Track Properties -- or, for
 * SUBSCRIBE_TRACKS, whether the resulting PUBLISH messages do (§10.2.21).
 *
 * The value is a uint8 under an odd Type, so the wire is Length=1 plus one byte;
 * it is not a varint that happens to be small. Omitted means 1, and 0 leaves the
 * Properties field present but empty rather than removing it.
 *
 * Two malformations, two error codes, and the draft draws the line between them:
 * a Length other than 1 is §1.4.3's "the Value does not match the serialization
 * defined by that Type" (KEY_VALUE_FORMATTING_ERROR), while a one-byte value
 * outside 0..1 is §10.2.21's own rule (PROTOCOL_VIOLATION).
 */
export class IncludeProperties implements Parameter {
  static readonly TYPE = MessageParameterType.IncludeProperties

  constructor(public readonly include: boolean) {}

  toKeyValuePair(): KeyValuePair {
    // `tryNewBytes`, not `newBytes`: 0x35 is odd, so the parity rule already wants
    // a Length here and no exception is needed.
    return KeyValuePair.tryNewBytes(IncludeProperties.TYPE, new Uint8Array([this.include ? 1 : 0]))
  }

  static fromKeyValuePair(pair: KeyValuePair): IncludeProperties | undefined {
    if (Number(pair.typeValue) !== IncludeProperties.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    if (pair.value.length !== 1) {
      throw new KeyValueFormattingError('IncludeProperties.fromKeyValuePair')
    }
    const value = pair.value[0]
    if (value !== 0 && value !== 1) {
      throw new ProtocolViolationError(
        'IncludeProperties.fromKeyValuePair',
        `INCLUDE_PROPERTIES must be 0 or 1, got ${value}`,
      )
    }
    return new IncludeProperties(value === 1)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('IncludeProperties', () => {
    test.each([true, false])('roundtrips include=%s', (include) => {
      const pair = new IncludeProperties(include).toKeyValuePair()
      expect(IncludeProperties.fromKeyValuePair(pair)?.include).toBe(include)
    })

    test('is one length-prefixed byte on the wire', () => {
      const pair = new IncludeProperties(true).toKeyValuePair()
      expect(pair.value).toEqual(new Uint8Array([1]))
      expect(new IncludeProperties(false).toKeyValuePair().value).toEqual(new Uint8Array([0]))
    })

    test.each([0, 2])('a Length of %i is a key-value formatting error', (length) => {
      const pair = KeyValuePair.tryNewBytes(MessageParameterType.IncludeProperties, new Uint8Array(length))
      expect(() => IncludeProperties.fromKeyValuePair(pair)).toThrow(KeyValueFormattingError)
    })

    test('a one-byte value outside 0 or 1 is a protocol violation', () => {
      const pair = KeyValuePair.tryNewBytes(MessageParameterType.IncludeProperties, new Uint8Array([2]))
      expect(() => IncludeProperties.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })

    test('returns undefined for another type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.Forward, 1n)
      expect(IncludeProperties.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
