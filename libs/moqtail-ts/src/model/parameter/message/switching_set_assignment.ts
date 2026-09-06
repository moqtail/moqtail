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
import { ProtocolViolationError } from '../../error/error'

/**
 * SWITCHING_SET_ASSIGNMENT (draft-wilaw-moq-moqt-ssts): assigns a
 * subscription to an SSTS switching set. Id 0 is the default algorithm of
 * Section 6.3.1; other ids select implementation-specific algorithms.
 *
 * The value is five varints — set id, algorithm id, throughput threshold
 * (kbps), throughput weight (1-10), activation threshold — followed by one
 * optional SET_RANK byte (degradation priority, lower is protected first,
 * default 0). The parameter may appear in SUBSCRIBE, REQUEST_UPDATE, or
 * PUBLISH_OK messages (Section 5).
 */
export class SwitchingSetAssignment implements Parameter {
  static readonly TYPE = MessageParameterType.SwitchingSetAssignment

  constructor(
    public readonly switchingSetId: bigint,
    public readonly algorithmId: bigint,
    public readonly throughputThresholdKbps: bigint,
    public readonly setThroughputWeight: bigint,
    public readonly activateSwitching: bigint,
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
        'SwitchingSetAssignment.fromKeyValuePair',
        `SET THROUGHPUT WEIGHT must be 1-10, got ${setThroughputWeight}`,
      )
    }
    const activateSwitching = buf.getVI()
    const setRank = buf.remaining > 0 ? buf.getU8() : 0
    if (buf.remaining > 0) {
      throw new ProtocolViolationError(
        'SwitchingSetAssignment.fromKeyValuePair',
        'Excess bytes in SWITCHING_SET_ASSIGNMENT parameter',
      )
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
  const { test, expect } = import.meta.vitest

  test('roundtrips all fields', () => {
    const orig = new SwitchingSetAssignment(7n, 0n, 2000n, 5n, 2n, 2)
    const parsed = SwitchingSetAssignment.fromKeyValuePair(orig.toKeyValuePair())
    expect(parsed?.switchingSetId).toBe(7n)
    expect(parsed?.algorithmId).toBe(0n)
    expect(parsed?.throughputThresholdKbps).toBe(2000n)
    expect(parsed?.setThroughputWeight).toBe(5n)
    expect(parsed?.activateSwitching).toBe(2n)
    expect(parsed?.setRank).toBe(2)
  })

  test('roundtrips the minimal encoding (default rank 0)', () => {
    const orig = new SwitchingSetAssignment(0n, 0n, 0n, 1n, 0n, 0)
    const parsed = SwitchingSetAssignment.fromKeyValuePair(orig.toKeyValuePair())
    expect(parsed).toEqual(orig)
  })

  test('fromKeyValuePair throws when the weight is out of range', () => {
    const buf = new ByteBuffer()
    buf.putVI(1n)
    buf.putVI(0n)
    buf.putVI(100n)
    buf.putVI(11n) // weight out of 1-10
    const pair = KeyValuePair.tryNewBytes(SwitchingSetAssignment.TYPE, buf.toUint8Array())
    expect(() => SwitchingSetAssignment.fromKeyValuePair(pair)).toThrow(ProtocolViolationError)
  })

  test('fromKeyValuePair throws on excess bytes', () => {
    const orig = new SwitchingSetAssignment(0n, 0n, 0n, 1n, 0n, 0)
    const pair = orig.toKeyValuePair()
    if (!(pair.value instanceof Uint8Array)) throw new Error('expected a bytes pair')
    const value = new Uint8Array(pair.value.length + 1)
    value.set(pair.value)
    const padded = KeyValuePair.tryNewBytes(SwitchingSetAssignment.TYPE, value)
    expect(() => SwitchingSetAssignment.fromKeyValuePair(padded)).toThrow(ProtocolViolationError)
  })

  test('fromKeyValuePair returns undefined for wrong type', () => {
    const pair = KeyValuePair.tryNewVarInt(MessageParameterType.ObjectDeliveryTimeout, 100n)
    expect(SwitchingSetAssignment.fromKeyValuePair(pair)).toBeUndefined()
  })
}
