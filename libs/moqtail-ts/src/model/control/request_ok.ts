/**
 * Copyright 2026 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { ByteBuffer, FrozenByteBuffer } from '../common/byte_buffer'
import { KeyValuePair } from '../common/pair'
import { ControlMessageType } from './constant'
import { LengthExceedsMaxError } from '../error/error'

export class RequestOk {
  public readonly requestId: bigint
  public readonly parameters: KeyValuePair[]

  constructor(requestId: bigint | number, parameters: KeyValuePair[] = []) {
    this.requestId = BigInt(requestId)
    this.parameters = parameters
  }

  getType(): ControlMessageType {
    return ControlMessageType.RequestOk
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.RequestOk)

    const payload = new ByteBuffer()
    payload.putVI(this.requestId)
    payload.putVI(BigInt(this.parameters.length))

    for (const param of this.parameters) {
      payload.putBytes(param.serialize().toUint8Array())
    }

    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('RequestOk::serialize(payloadBytes)', 0xffff, payloadBytes.length)
    }

    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)

    return buf.freeze()
  }

  static parsePayload(frozen: FrozenByteBuffer): RequestOk {
    const requestId = frozen.getVI()
    const numParams = Number(frozen.getVI())
    const parameters: KeyValuePair[] = []

    for (let i = 0; i < numParams; i++) {
      parameters.push(KeyValuePair.deserialize(frozen))
    }

    return new RequestOk(requestId, parameters)
  }
}
