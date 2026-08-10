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
import { SetupOptionType } from '../constant'
import { Parameter } from '../parameter'

/**
 * Free-form implementation identification string, e.g. for logging and debugging
 * (draft-18 §10.3.1.5). Either peer may send it.
 */
export class MoqtImplementation implements Parameter {
  static readonly TYPE = SetupOptionType.MoqtImplementation
  constructor(public readonly info: string) {}

  toKeyValuePair(): KeyValuePair {
    const bytes = new TextEncoder().encode(this.info)
    return KeyValuePair.tryNewBytes(MoqtImplementation.TYPE, bytes)
  }

  static fromKeyValuePair(pair: KeyValuePair): MoqtImplementation | undefined {
    if (Number(pair.typeValue) !== MoqtImplementation.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const info = new TextDecoder().decode(pair.value)
    return new MoqtImplementation(info)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MoqtImplementation', () => {
    test('fromKeyValuePair returns instance for valid pair', () => {
      const pair = new MoqtImplementation('moqtail-ts/0.1').toKeyValuePair()
      const param = MoqtImplementation.fromKeyValuePair(pair)
      expect(param).toBeInstanceOf(MoqtImplementation)
      expect(param?.info).toBe('moqtail-ts/0.1')
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(SetupOptionType.MaxRequestId, 1n)
      const param = MoqtImplementation.fromKeyValuePair(pair)
      expect(param).toBeUndefined()
    })
  })
}
