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

import { KeyValuePair } from '@/model/common/pair'
import { LOCPropertyId } from './constant'

const utf8 = new TextDecoder('utf-8', { fatal: true })

/** @public */
export class CodecString {
  static readonly TYPE = LOCPropertyId.CodecString

  constructor(public readonly codec: string) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewBytes(CodecString.TYPE, new TextEncoder().encode(this.codec))
  }

  static fromKeyValuePair(pair: KeyValuePair): CodecString | undefined {
    const type = Number(pair.typeValue)
    if (type !== CodecString.TYPE || !(pair.value instanceof Uint8Array)) {
      return undefined
    }
    try {
      return new CodecString(utf8.decode(pair.value))
    } catch (error) {
      if (error instanceof TypeError) {
        return undefined
      }
      throw error
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  describe('CodecStringProperty', () => {
    test('should roundtrip CodecString', () => {
      const pair = new CodecString('avc3.42E01E').toKeyValuePair()
      expect(CodecString.fromKeyValuePair(pair)?.codec).toBe('avc3.42E01E')
    })

    test('should return undefined for invalid UTF-8', () => {
      const pair = KeyValuePair.tryNewBytes(LOCPropertyId.CodecString, new Uint8Array([0xff]))
      expect(CodecString.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
