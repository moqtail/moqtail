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
 * SSTS_ALGORITHMS setup option: the SSTS algorithms this endpoint supports
 * (draft-wilaw-moq-moqt-ssts, Section 3.1). The value is a list of algorithm
 * ids, each a varint. An empty list — or the absence of this option —
 * prohibits the use of SSTS.
 */
export class SstsAlgorithms implements Parameter {
  static readonly TYPE = SetupOptionType.SstsAlgorithms

  constructor(public readonly algorithms: readonly bigint[] = []) {}

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
  const { test, expect } = import.meta.vitest

  test('roundtrips an algorithm list', () => {
    const orig = new SstsAlgorithms([0n, 1n])
    const parsed = SstsAlgorithms.fromKeyValuePair(orig.toKeyValuePair())
    expect(parsed?.algorithms).toEqual([0n, 1n])
  })

  test('roundtrips an empty list (SSTS not supported)', () => {
    const orig = new SstsAlgorithms([])
    const parsed = SstsAlgorithms.fromKeyValuePair(orig.toKeyValuePair())
    expect(parsed?.algorithms).toEqual([])
  })

  test('fromKeyValuePair returns undefined for wrong type', () => {
    const pair = KeyValuePair.tryNewVarInt(SetupOptionType.MaxAuthTokenCacheSize, 1n)
    expect(SstsAlgorithms.fromKeyValuePair(pair)).toBeUndefined()
  })
}
