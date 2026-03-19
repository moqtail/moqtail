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

import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from '../common/byte_buffer'
import { KeyValuePair } from '../common/pair'
import { ProtocolViolationError } from '../error/error'
import { TrackExtensionType } from './constant'

export class ImmutableExtensionsObjectExtension {
  static readonly TYPE = TrackExtensionType.ImmutableExtensions

  constructor(public readonly extensions: KeyValuePair[]) {}

  toKeyValuePair(): KeyValuePair {
    const inner = new ByteBuffer()
    for (const kvp of this.extensions) {
      inner.putBytes(kvp.serialize().toUint8Array())
    }
    return KeyValuePair.tryNewBytes(ImmutableExtensionsObjectExtension.TYPE, inner.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): ImmutableExtensionsObjectExtension | undefined {
    if (Number(pair.typeValue) !== ImmutableExtensionsObjectExtension.TYPE || !(pair.value instanceof Uint8Array))
      return undefined
    const buf = new FrozenByteBuffer(pair.value)
    const extensions: KeyValuePair[] = []
    while (buf.remaining > 0) {
      const inner = KeyValuePair.deserialize(buf)
      if (Number(inner.typeValue) === TrackExtensionType.ImmutableExtensions) {
        throw new ProtocolViolationError(
          'ImmutableExtensionsObjectExtension.fromKeyValuePair',
          'ImmutableExtensions must not contain nested ImmutableExtensions (0x0B)',
        )
      }
      extensions.push(inner)
    }
    return new ImmutableExtensionsObjectExtension(extensions)
  }
}

export class PriorGroupIdGapExtension {
  static readonly TYPE = TrackExtensionType.PriorGroupIdGap

  constructor(public readonly gap: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(PriorGroupIdGapExtension.TYPE, this.gap)
  }

  static fromKeyValuePair(pair: KeyValuePair): PriorGroupIdGapExtension | undefined {
    if (Number(pair.typeValue) !== PriorGroupIdGapExtension.TYPE || typeof pair.value !== 'bigint') return undefined
    return new PriorGroupIdGapExtension(pair.value)
  }
}

export class PriorObjectIdGapExtension {
  static readonly TYPE = TrackExtensionType.PriorObjectIdGap

  constructor(public readonly gap: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(PriorObjectIdGapExtension.TYPE, this.gap)
  }

  static fromKeyValuePair(pair: KeyValuePair): PriorObjectIdGapExtension | undefined {
    if (Number(pair.typeValue) !== PriorObjectIdGapExtension.TYPE || typeof pair.value !== 'bigint') return undefined
    return new PriorObjectIdGapExtension(pair.value)
  }
}

export class UnknownObjectExtension {
  constructor(public readonly kvp: KeyValuePair) {}

  toKeyValuePair(): KeyValuePair {
    return this.kvp
  }
}

export type ObjectExtension =
  | ImmutableExtensionsObjectExtension
  | PriorGroupIdGapExtension
  | PriorObjectIdGapExtension
  | UnknownObjectExtension

export namespace ObjectExtension {
  export function fromKeyValuePair(pair: KeyValuePair): ObjectExtension {
    return (
      ImmutableExtensionsObjectExtension.fromKeyValuePair(pair) ??
      PriorGroupIdGapExtension.fromKeyValuePair(pair) ??
      PriorObjectIdGapExtension.fromKeyValuePair(pair) ??
      new UnknownObjectExtension(pair)
    )
  }

  export function toKeyValuePair(ext: ObjectExtension): KeyValuePair {
    return ext.toKeyValuePair()
  }

  export function deserializeAll(buf: BaseByteBuffer): ObjectExtension[] {
    const result: ObjectExtension[] = []
    while (buf.remaining > 0) {
      const kvp = KeyValuePair.deserialize(buf)
      result.push(fromKeyValuePair(kvp))
    }
    return result
  }

  export function serializeInto(exts: ObjectExtension[], payload: ByteBuffer): void {
    for (const ext of exts) {
      payload.putBytes(ext.toKeyValuePair().serialize().toUint8Array())
    }
  }

  export function isImmutableExtensions(ext: ObjectExtension): ext is ImmutableExtensionsObjectExtension {
    return ext instanceof ImmutableExtensionsObjectExtension
  }

  export function isPriorGroupIdGap(ext: ObjectExtension): ext is PriorGroupIdGapExtension {
    return ext instanceof PriorGroupIdGapExtension
  }

  export function isPriorObjectIdGap(ext: ObjectExtension): ext is PriorObjectIdGapExtension {
    return ext instanceof PriorObjectIdGapExtension
  }

  export function isUnknown(ext: ObjectExtension): ext is UnknownObjectExtension {
    return ext instanceof UnknownObjectExtension
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  function roundtrip(exts: ObjectExtension[]): ObjectExtension[] {
    const buf = new ByteBuffer()
    ObjectExtension.serializeInto(exts, buf)
    return ObjectExtension.deserializeAll(buf.freeze())
  }

  describe('ObjectExtension', () => {
    test('empty deserializeAll returns []', () => {
      const buf = new FrozenByteBuffer(new Uint8Array())
      expect(ObjectExtension.deserializeAll(buf)).toEqual([])
    })

    test('PriorGroupIdGapExtension roundtrip', () => {
      const ext = new PriorGroupIdGapExtension(7n)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(PriorGroupIdGapExtension)
      expect((result as PriorGroupIdGapExtension).gap).toBe(7n)
    })

    test('PriorObjectIdGapExtension roundtrip', () => {
      const ext = new PriorObjectIdGapExtension(3n)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(PriorObjectIdGapExtension)
      expect((result as PriorObjectIdGapExtension).gap).toBe(3n)
    })

    test('ImmutableExtensionsObjectExtension roundtrip', () => {
      const inner = [KeyValuePair.tryNewVarInt(0x04, 100n)]
      const ext = new ImmutableExtensionsObjectExtension(inner)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(ImmutableExtensionsObjectExtension)
      expect((result as ImmutableExtensionsObjectExtension).extensions.length).toBe(1)
    })

    test('ImmutableExtensionsObjectExtension throws on nested 0x0B', () => {
      const nested = new ImmutableExtensionsObjectExtension([])
      const innerKvp = nested.toKeyValuePair()
      const outer = new ByteBuffer()
      outer.putBytes(innerKvp.serialize().toUint8Array())
      const outerKvp = KeyValuePair.tryNewBytes(TrackExtensionType.ImmutableExtensions, outer.toUint8Array())
      expect(() => ImmutableExtensionsObjectExtension.fromKeyValuePair(outerKvp)).toThrow(ProtocolViolationError)
    })

    test('UnknownObjectExtension pass-through roundtrip', () => {
      const kvp = KeyValuePair.tryNewVarInt(0x02, 42n)
      const ext = new UnknownObjectExtension(kvp)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(UnknownObjectExtension)
      expect((result as UnknownObjectExtension).kvp.typeValue).toBe(0x02n)
    })

    test('mixed list roundtrip', () => {
      const exts: ObjectExtension[] = [
        new PriorGroupIdGapExtension(5n),
        new PriorObjectIdGapExtension(2n),
        new ImmutableExtensionsObjectExtension([]),
      ]
      const result = roundtrip(exts)
      expect(result.length).toBe(3)
      expect(result[0]).toBeInstanceOf(PriorGroupIdGapExtension)
      expect(result[1]).toBeInstanceOf(PriorObjectIdGapExtension)
      expect(result[2]).toBeInstanceOf(ImmutableExtensionsObjectExtension)
    })
  })
}
