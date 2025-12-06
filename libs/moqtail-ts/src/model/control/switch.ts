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

import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from '../common/byte_buffer'
import { KeyValuePair } from '../common/pair'
import { ControlMessageType } from './constant'
import { FullTrackName } from '../data'

export class Switch {
  constructor(
    public requestId: bigint,
    public fullTrackName: FullTrackName,
    public subscriptionRequestId: bigint,
    public parameters: KeyValuePair[],
  ) {}

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.Switch)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putBytes(this.fullTrackName.serialize().toUint8Array())
    payload.putVI(this.subscriptionRequestId)

    payload.putVI(this.parameters.length)
    for (const param of this.parameters) {
      payload.putBytes(param.serialize().toUint8Array())
    }

    const payloadBytes = payload.toUint8Array()
    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)

    return buf.freeze()
  }

  static parsePayload(buf: BaseByteBuffer): Switch {
    const requestId = buf.getVI()
    const fullTrackName = buf.getFullTrackName()
    const subscriptionRequestId = buf.getVI()

    const paramCount = Number(buf.getVI())
    const parameters: KeyValuePair[] = []
    for (let i = 0; i < paramCount; i++) {
      parameters.push(KeyValuePair.deserialize(buf))
    }

    return new Switch(requestId, fullTrackName, subscriptionRequestId, parameters)
  }
}
