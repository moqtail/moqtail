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

import { ByteBuffer } from '../../common'
import { KeyValuePair } from '../../common/pair'
import { SetupOptionType } from '../constant'
import { Parameter } from '../parameter'

/**
 * The sender-side track switching (SSTS) algorithms this endpoint supports.
 *
 * PROVISIONAL: sender-side track switching is not an adopted extension, so
 * this option has no codepoint in the registry the conformance fixture tracks
 * (`dev/conformance/draft22`) and is never asserted against it. An empty list
 * — or the absence of the option — prohibits the use of SSTS.
 */
export class SstsAlgorithms implements Parameter {
  static readonly TYPE = SetupOptionType.SstsAlgorithms

  constructor(public readonly algorithms: bigint[]) {}

  toKeyValuePair(): KeyValuePair {
    const buf = new ByteBuffer()
    for (const algorithm of this.algorithms) buf.putVI(algorithm)
    return KeyValuePair.tryNewBytes(SstsAlgorithms.TYPE, buf.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): SstsAlgorithms | undefined {
    if (Number(pair.typeValue) !== SstsAlgorithms.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    const algorithms: bigint[] = []
    while (buf.remaining > 0) algorithms.push(buf.getVI())
    return new SstsAlgorithms(algorithms)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SstsAlgorithms', () => {
    test('roundtrips a list of algorithm ids', () => {
      const orig = new SstsAlgorithms([0n, 0xff00n])
      const parsed = SstsAlgorithms.fromKeyValuePair(orig.toKeyValuePair())
      expect(parsed).toBeInstanceOf(SstsAlgorithms)
      expect(parsed?.algorithms).toEqual([0n, 0xff00n])
    })
    test('an empty list roundtrips and prohibits SSTS', () => {
      const orig = new SstsAlgorithms([])
      const parsed = SstsAlgorithms.fromKeyValuePair(orig.toKeyValuePair())
      expect(parsed?.algorithms).toEqual([])
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(SetupOptionType.MaxAuthTokenCacheSize, 1n)
      expect(SstsAlgorithms.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
