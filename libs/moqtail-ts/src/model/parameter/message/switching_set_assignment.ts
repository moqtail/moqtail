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
import { ProtocolViolationError } from '../../error/error'
import { MessageParameterType } from '../constant'
import { Parameter } from '../parameter'

/**
 * Assigns a subscription to an SSTS (sender-side track switching) switching
 * set (draft-wilaw-moq-moqt-ssts, Section 5). Carried on SUBSCRIBE and on the
 * PUBLISH_OK of a pushed track.
 *
 * PROVISIONAL: this parameter comes from an unadopted draft and has no
 * codepoint in the adopted draft-18 registry (`dev/conformance/draft18`), so
 * it is never asserted against that fixture. Id 0 is the draft's default
 * algorithm (Section 6.3.1); other ids select implementation-specific
 * algorithms.
 */
export class SwitchingSetAssignment implements Parameter {
  static readonly TYPE = MessageParameterType.SwitchingSetAssignment

  constructor(
    public readonly switchingSetId: bigint,
    public readonly algorithmId: bigint,
    public readonly throughputThresholdKbps: bigint,
    /** Relative bandwidth weight among same-rank sets, 1 through 10. */
    public readonly setThroughputWeight: bigint,
    /** 0 pauses SSTS for the set; switching activates once the number of assigned tracks reaches this value. */
    public readonly activateSwitching: bigint,
    /** Degradation priority; lower values are protected first. Default 0. */
    public readonly setRank: number = 0,
  ) {}

  toKeyValuePair(): KeyValuePair {
    const buf = new ByteBuffer()
    buf.putVI(this.switchingSetId)
    buf.putVI(this.algorithmId)
    buf.putVI(this.throughputThresholdKbps)
    buf.putVI(this.setThroughputWeight)
    buf.putVI(this.activateSwitching)
    buf.putU8(this.setRank)
    return KeyValuePair.tryNewBytes(SwitchingSetAssignment.TYPE, buf.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): SwitchingSetAssignment | undefined {
    if (Number(pair.typeValue) !== SwitchingSetAssignment.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    const switchingSetId = buf.getVI()
    const algorithmId = buf.getVI()
    const throughputThresholdKbps = buf.getVI()
    const setThroughputWeight = buf.getVI()
    if (setThroughputWeight < 1n || setThroughputWeight > 10n) {
      throw new ProtocolViolationError(
        'SwitchingSetAssignment',
        `SET THROUGHPUT WEIGHT must be 1-10, got ${setThroughputWeight}`,
      )
    }
    const activateSwitching = buf.getVI()
    const setRank = buf.remaining > 0 ? buf.getU8() : 0
    if (buf.remaining > 0) {
      throw new ProtocolViolationError('SwitchingSetAssignment', 'excess bytes in SWITCHING_SET_ASSIGNMENT parameter')
    }
    return new SwitchingSetAssignment(
      switchingSetId,
      algorithmId,
      throughputThresholdKbps,
      setThroughputWeight,
      activateSwitching,
      setRank,
    )
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SwitchingSetAssignment', () => {
    const params = () => new SwitchingSetAssignment(7n, 0xff00n, 2500n, 3n, 2n, 1)

    test('roundtrips all fields', () => {
      const parsed = SwitchingSetAssignment.fromKeyValuePair(params().toKeyValuePair())
      expect(parsed).toBeInstanceOf(SwitchingSetAssignment)
      expect(parsed?.switchingSetId).toBe(7n)
      expect(parsed?.algorithmId).toBe(0xff00n)
      expect(parsed?.throughputThresholdKbps).toBe(2500n)
      expect(parsed?.setThroughputWeight).toBe(3n)
      expect(parsed?.activateSwitching).toBe(2n)
      expect(parsed?.setRank).toBe(1)
    })
    test('a missing rank byte defaults to 0', () => {
      const pair = new SwitchingSetAssignment(1n, 0n, 0n, 1n, 0n).toKeyValuePair()
      if (!(pair.value instanceof Uint8Array)) throw new Error('expected a bytes parameter')
      // Strip the trailing rank byte the serializer always writes.
      const raw = KeyValuePair.tryNewBytes(SwitchingSetAssignment.TYPE, pair.value.slice(0, -1))
      const parsed = SwitchingSetAssignment.fromKeyValuePair(raw)
      expect(parsed?.setRank).toBe(0)
    })
    test('a weight outside 1-10 is a protocol violation', () => {
      const pair = new SwitchingSetAssignment(1n, 0n, 0n, 11n, 0n).toKeyValuePair()
      expect(() => SwitchingSetAssignment.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
    })
    test('excess trailing bytes are a protocol violation', () => {
      const full = params().toKeyValuePair()
      const extra = new Uint8Array([...(full.value as Uint8Array), 1])
      const raw = KeyValuePair.tryNewBytes(SwitchingSetAssignment.TYPE, extra)
      expect(() => SwitchingSetAssignment.fromKeyValuePair(raw)).toThrow(ProtocolViolationError)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.Forward, 1n)
      expect(SwitchingSetAssignment.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
