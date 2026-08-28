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
import { SwitchMode } from '../../control/constant'
import { ProtocolViolationError } from '../../error/error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'

/**
 * Identifies the subscription being replaced by a switch request.
 * The value is two varints followed by a flags byte: bit 7 (MSB) is Publish Done;
 * bits 0 through 6 are reserved and must be zero.
 */
export class SwitchFrom implements Parameter {
  static readonly TYPE = MessageParameterType.SwitchFrom

  constructor(
    public readonly requestId: bigint,
    public readonly mode: SwitchMode,
    public readonly publishDone: boolean,
  ) {}

  toKeyValuePair(): KeyValuePair {
    const buf = new ByteBuffer()
    buf.putVI(this.requestId)
    buf.putVI(this.mode)
    buf.putU8(this.publishDone ? 1 << 7 : 0)
    return KeyValuePair.tryNewBytes(SwitchFrom.TYPE, buf.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): SwitchFrom | undefined {
    if (Number(pair.typeValue) !== SwitchFrom.TYPE || !(pair.value instanceof Uint8Array)) return undefined

    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    const requestId = buf.getVI()
    const modeValue = buf.getVI()
    if (modeValue !== BigInt(SwitchMode.Hard) && modeValue !== BigInt(SwitchMode.Soft)) {
      throw new ProtocolViolationError('SwitchFrom.fromKeyValuePair', `invalid mode ${modeValue}`)
    }
    const flags = buf.getU8()
    if ((flags & 0x7f) !== 0) {
      throw new ProtocolViolationError('SwitchFrom.fromKeyValuePair', `reserved bits must be zero, got ${flags}`)
    }
    if (buf.remaining !== 0) {
      throw new ProtocolViolationError('SwitchFrom.fromKeyValuePair', 'unexpected trailing bytes')
    }
    return new SwitchFrom(requestId, Number(modeValue) as SwitchMode, (flags & 0x80) !== 0)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SwitchFrom', () => {
    test('roundtrips correctly', () => {
      const orig = new SwitchFrom(128242n, SwitchMode.Soft, true)
      expect(SwitchFrom.fromKeyValuePair(orig.toKeyValuePair())).toEqual(orig)
    })

    test('rejects non-zero reserved bits', () => {
      const buf = new ByteBuffer()
      buf.putVI(1n)
      buf.putVI(SwitchMode.Hard)
      buf.putU8(0x01)
      const pair = KeyValuePair.tryNewBytes(SwitchFrom.TYPE, buf.toUint8Array())
      expect(() => SwitchFrom.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })
  })
}
