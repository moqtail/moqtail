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
import { FrozenByteBuffer, BaseByteBuffer } from '../common/byte_buffer'
import { InvalidTypeError } from '../error'

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
    switch (type) {
      case 0x05:
        return FetchHeader.deserialize(buf)
      case 0x10:
      case 0x11:
      case 0x12:
      case 0x13:
      case 0x14:
      case 0x15:
      case 0x18:
      case 0x19:
      case 0x1a:
      case 0x1b:
      case 0x1c:
      case 0x1d:
        return SubgroupHeader.deserialize(buf)
      default:
        throw new InvalidTypeError('Header::deserialize(type)', `Unknown header type: ${type}`)
    }
  }
}
