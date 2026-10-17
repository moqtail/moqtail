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

import { KeyValuePair } from '../../common/pair'
import { SetupOptionType } from '../constant'
import { Parameter } from '../parameter'

export class Path implements Parameter {
  static readonly TYPE = SetupOptionType.Path
  constructor(public readonly moqtPath: string) {}

  toKeyValuePair(): KeyValuePair {
    const encoder = new TextEncoder()
    const bytes = encoder.encode(this.moqtPath)
    return KeyValuePair.tryNewBytes(Path.TYPE, bytes)
  }

  static fromKeyValuePair(pair: KeyValuePair): Path | undefined {
    if (Number(pair.typeValue) !== Path.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const moqtPath = new TextDecoder().decode(pair.value)
    return new Path(moqtPath)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('PathParameter', () => {
    test('fromKeyValuePair returns instance for valid pair', () => {
      const pair = new Path('abc').toKeyValuePair()
      const param = Path.fromKeyValuePair(pair)
      expect(param).toBeInstanceOf(Path)
      expect(param?.moqtPath).toBe('abc')
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(SetupOptionType.MaxAuthTokenCacheSize, 1n)
      const param = Path.fromKeyValuePair(pair)
      expect(param).toBeUndefined()
    })
  })
}
