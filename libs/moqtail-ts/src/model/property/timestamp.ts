/**
 * Copyright 2025 The MOQtail Authors
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
import { LOCPropertyId } from './constant'

/** @public In Timescale units when one applies, else wall-clock µs since the Unix epoch (LOC 2.3.1.1). */
export class Timestamp {
  static readonly TYPE = LOCPropertyId.Timestamp
  constructor(public readonly timestamp: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(Timestamp.TYPE, this.timestamp)
  }

  /** `timescale` is the object's Timescale, else the track's (LOC 2.3.1.2). */
  toMicros(timescale?: bigint): number {
    if (timescale === undefined) {
      return Number(this.timestamp)
    }
    // Multiply in bigint: a 90 kHz media time times 1e6 exceeds 2^53
    return Number((this.timestamp * 1_000_000n) / timescale)
  }

  static fromKeyValuePair(pair: KeyValuePair): Timestamp | undefined {
    const type = Number(pair.typeValue)
    if (type === Timestamp.TYPE && typeof pair.value === 'bigint') {
      return new Timestamp(pair.value)
    }
    return undefined
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('TimestampProperty', () => {
    test('should roundtrip Timestamp', () => {
      const value = 1234567890123456789n
      const pair = new Timestamp(value).toKeyValuePair()
      const header = Timestamp.fromKeyValuePair(pair)
      expect(header).toBeInstanceOf(Timestamp)
      expect(header?.timestamp).toBe(value)
    })

    test('toMicros returns the value as is without a timescale', () => {
      expect(new Timestamp(1_700_000_000_000_000n).toMicros()).toBe(1_700_000_000_000_000)
    })

    test('toMicros converts from the timescale', () => {
      expect(new Timestamp(90_000n).toMicros(90_000n)).toBe(1_000_000)
    })

    test('toMicros stays exact for an epoch-sized 90 kHz value', () => {
      expect(new Timestamp(153_000_000_000_198n).toMicros(90_000n)).toBe(1_700_000_000_002_200)
    })
  })
}
