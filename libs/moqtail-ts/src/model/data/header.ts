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

import { FetchHeader } from './fetch_header'
import { SubgroupHeader } from './subgroup_header'
import { ByteBuffer, FrozenByteBuffer, BaseByteBuffer } from '../common/byte_buffer'
import { ProtocolViolationError } from '../error'
import { SubgroupHeaderType } from './constant'

export type Header = FetchHeader | SubgroupHeader

export namespace Header {
  export function newFetch(...args: ConstructorParameters<typeof FetchHeader>): Header {
    return new FetchHeader(...args)
  }
  export function newSubgroup(...args: ConstructorParameters<typeof SubgroupHeader>): Header {
    return new SubgroupHeader(...args)
  }
  export function isFetch(header: Header): header is FetchHeader {
    return header instanceof FetchHeader
  }
  export function isSubgroup(header: Header): header is SubgroupHeader {
    return header instanceof SubgroupHeader
  }
  export function serialize(header: Header): FrozenByteBuffer {
    return header.serialize()
  }
  export function deserialize(buf: BaseByteBuffer): Header {
    buf.checkpoint()
    const type = buf.getNumberVI()
    buf.restore()
    if (type === 0x05) {
      return FetchHeader.deserialize(buf)
    } else if ((type & SubgroupHeaderType.REQUIRED_BIT) !== 0 && (type & SubgroupHeaderType.INVALID_BITS_MASK) === 0) {
      return SubgroupHeader.deserialize(buf)
    } else {
      throw new ProtocolViolationError(
        'Header::deserialize(type)',
        `unknown data stream header type 0x${type.toString(16)}`,
      )
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('Header', () => {
    test('rejects an unknown data stream header type as a protocol violation', () => {
      const buf = new ByteBuffer()
      buf.putVI(0x06)
      expect(() => Header.deserialize(buf.freeze())).toThrow(ProtocolViolationError)
    })
  })
}
