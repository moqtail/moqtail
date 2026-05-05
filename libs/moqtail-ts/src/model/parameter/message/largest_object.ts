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
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'
import { Location } from '../../common'

/**
 * The largest Location in the Track observed by the sending endpoint.
 * If omitted from a message, the sending endpoint has not published or
 * received any Objects in the Track.
 */
export class LargestObject implements Parameter {
  static readonly TYPE = MessageParameterType.LargestObject

  constructor(public readonly location: Location) {}

  toKeyValuePair(): KeyValuePair {
    const buf = new ByteBuffer()
    buf.putVI(this.location.group)
    buf.putVI(this.location.object)
    return KeyValuePair.tryNewBytes(LargestObject.TYPE, buf.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): LargestObject | undefined {
    if (Number(pair.typeValue) !== LargestObject.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    const location = buf.getLocation()
    return new LargestObject(location)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('LargestObject', () => {
    test('roundtrips correctly', () => {
      const orig = new LargestObject(new Location(10n, 5n))
      const pair = orig.toKeyValuePair()
      const parsed = LargestObject.fromKeyValuePair(pair)
      expect(parsed).toBeInstanceOf(LargestObject)
      expect(parsed?.location.group).toBe(10n)
      expect(parsed?.location.object).toBe(5n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.DeliveryTimeout, 100n)
      expect(LargestObject.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
