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
 * Top-N track filter parameter (moq-transport PR #1401).
 * Used in SUBSCRIBE_NAMESPACE to request relay-side Top-N ranking.
 *
 * Wire encoding: single packed VarInt = `(propertyType << 8n) | BigInt(maxSelected)`.
 * Example: propertyType=0x12n (speech-activity VAD), maxSelected=2 → value=0x1202n.
 * maxSelected occupies only the low 8 bits of the packed value, so it must be 0-255 —
 * a larger value would silently corrupt propertyType.
 */
export class TrackFilter implements Parameter {
  static readonly TYPE = MessageParameterType.TrackFilter

  constructor(
    public readonly propertyType: bigint,
    public readonly maxSelected: number,
  ) {
    if (!Number.isInteger(maxSelected) || maxSelected < 0 || maxSelected > 255) {
      throw new ProtocolViolationError(
        'TrackFilter.constructor',
        `maxSelected must be an integer 0-255, got ${maxSelected}`,
      )
    }
  }

  toKeyValuePair(): KeyValuePair {
    const packed = (this.propertyType << 8n) | BigInt(this.maxSelected)
    return KeyValuePair.tryNewVarInt(TrackFilter.TYPE, packed)
  }

  static fromKeyValuePair(pair: KeyValuePair): TrackFilter | undefined {
    if (Number(pair.typeValue) !== TrackFilter.TYPE || typeof pair.value !== 'bigint') return undefined
    const propertyType = pair.value >> 8n
    const maxSelected = Number(pair.value & 0xffn)
    return new TrackFilter(propertyType, maxSelected)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('TrackFilter', () => {
    test('roundtrips speech-activity top-2', () => {
      const orig = new TrackFilter(0x12n, 2)
      const pair = orig.toKeyValuePair()
      const parsed = TrackFilter.fromKeyValuePair(pair)
      expect(parsed).toBeInstanceOf(TrackFilter)
      expect(parsed?.propertyType).toBe(0x12n)
      expect(parsed?.maxSelected).toBe(2)
    })
    test('packed value is (propertyType << 8) | maxSelected', () => {
      const param = new TrackFilter(0x12n, 2)
      const pair = param.toKeyValuePair()
      expect(pair.value).toBe(0x1202n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.Forward, 0x1202n)
      expect(TrackFilter.fromKeyValuePair(pair)).toBeUndefined()
    })
    test('accepts boundary values 0 and 255', () => {
      expect(new TrackFilter(0x12n, 0).maxSelected).toBe(0)
      expect(new TrackFilter(0x12n, 255).maxSelected).toBe(255)
    })
    test('constructor throws on maxSelected > 255', () => {
      expect(() => new TrackFilter(0x12n, 256)).toThrow(ProtocolViolationError)
    })
    test('constructor throws on negative or non-integer maxSelected', () => {
      expect(() => new TrackFilter(0x12n, -1)).toThrow(ProtocolViolationError)
      expect(() => new TrackFilter(0x12n, 1.5)).toThrow(ProtocolViolationError)
    })
  })
}
