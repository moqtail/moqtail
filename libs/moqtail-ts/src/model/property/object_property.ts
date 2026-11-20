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
import { greaseValue, isGrease } from '../common/grease'
import { KeyValuePair, deserializeKvpListUntilEmpty, serializeKvpList } from '../common/pair'
import { NotEnoughBytesError, ProtocolViolationError } from '../error/error'
import { PropertyRanges, TrackPropertyType } from './constant'

/** @public */
export class ImmutablePropertiesObjectProperty {
  static readonly TYPE = TrackPropertyType.ImmutableProperties

  constructor(public readonly properties: KeyValuePair[]) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewBytes(
      ImmutablePropertiesObjectProperty.TYPE,
      serializeKvpList(this.properties).toUint8Array(),
    )
  }

  static fromKeyValuePair(pair: KeyValuePair): ImmutablePropertiesObjectProperty | undefined {
    if (Number(pair.typeValue) !== ImmutablePropertiesObjectProperty.TYPE || !(pair.value instanceof Uint8Array))
      return undefined
    const buf = new FrozenByteBuffer(pair.value)
    const properties = deserializeKvpListUntilEmpty(buf)
    properties.forEach(rejectMandatoryTrackProperty)
    if (properties.some((inner) => Number(inner.typeValue) === TrackPropertyType.ImmutableProperties)) {
      throw new ProtocolViolationError(
        'ImmutablePropertiesObjectProperty.fromKeyValuePair',
        'ImmutableProperties must not contain nested ImmutableProperties (0x0B)',
      )
    }
    return new ImmutablePropertiesObjectProperty(properties)
  }
}

// The mandatory range is Track scope only; on an Object it can only be misplaced. The
// GREASE values that fall in it are left alone: a peer sends them to exercise
// unknown-value handling, which must never be what drops its track.
function rejectMandatoryTrackProperty(pair: KeyValuePair): void {
  const { from, to } = PropertyRanges.MandatoryTrack
  if (pair.typeValue >= from && pair.typeValue <= to && !isGrease(pair.typeValue)) {
    throw new ProtocolViolationError(
      'ObjectProperty.fromKeyValuePair',
      `Mandatory Track Property 0x${pair.typeValue.toString(16)} is not allowed on an Object`,
    )
  }
}

/** @public */
export class PriorGroupIdGapProperty {
  static readonly TYPE = TrackPropertyType.PriorGroupIdGap

  constructor(public readonly gap: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(PriorGroupIdGapProperty.TYPE, this.gap)
  }

  static fromKeyValuePair(pair: KeyValuePair): PriorGroupIdGapProperty | undefined {
    if (Number(pair.typeValue) !== PriorGroupIdGapProperty.TYPE || typeof pair.value !== 'bigint') return undefined
    return new PriorGroupIdGapProperty(pair.value)
  }
}

/** @public */
export class PriorObjectIdGapProperty {
  static readonly TYPE = TrackPropertyType.PriorObjectIdGap

  constructor(public readonly gap: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(PriorObjectIdGapProperty.TYPE, this.gap)
  }

  static fromKeyValuePair(pair: KeyValuePair): PriorObjectIdGapProperty | undefined {
    if (Number(pair.typeValue) !== PriorObjectIdGapProperty.TYPE || typeof pair.value !== 'bigint') return undefined
    return new PriorObjectIdGapProperty(pair.value)
  }
}

/** @public */
export class UnknownObjectProperty {
  constructor(public readonly kvp: KeyValuePair) {}

  toKeyValuePair(): KeyValuePair {
    return this.kvp
  }
}

/** @public */
export type ObjectProperty =
  ImmutablePropertiesObjectProperty | PriorGroupIdGapProperty | PriorObjectIdGapProperty | UnknownObjectProperty

/** @public */
export namespace ObjectProperty {
  export function fromKeyValuePair(pair: KeyValuePair): ObjectProperty {
    rejectMandatoryTrackProperty(pair)
    return (
      ImmutablePropertiesObjectProperty.fromKeyValuePair(pair) ??
      PriorGroupIdGapProperty.fromKeyValuePair(pair) ??
      PriorObjectIdGapProperty.fromKeyValuePair(pair) ??
      new UnknownObjectProperty(pair)
    )
  }

  /**
   * Reads an Object's length-delimited property block as raw pairs, checked like
   * `fromKeyValuePair`. Throws ProtocolViolationError if it is malformed.
   */
  export function deserializeRaw(buf: BaseByteBuffer): KeyValuePair[] {
    try {
      const pairs = deserializeKvpListUntilEmpty(buf)
      pairs.forEach(fromKeyValuePair)
      return pairs
    } catch (e) {
      // The block is complete, so running short is malformed data, not a reason to wait for more.
      if (e instanceof NotEnoughBytesError) throw new ProtocolViolationError('ObjectProperty.deserializeRaw', e.message)
      throw e
    }
  }

  export function toKeyValuePair(ext: ObjectProperty): KeyValuePair {
    return ext.toKeyValuePair()
  }

  export function deserializeAll(buf: BaseByteBuffer): ObjectProperty[] {
    return deserializeKvpListUntilEmpty(buf).map(fromKeyValuePair)
  }

  export function serializeInto(exts: ObjectProperty[], payload: ByteBuffer): void {
    payload.putBytes(serializeKvpList(exts.map((ext) => ext.toKeyValuePair())).toUint8Array())
  }

  export function isImmutableProperties(ext: ObjectProperty): ext is ImmutablePropertiesObjectProperty {
    return ext instanceof ImmutablePropertiesObjectProperty
  }

  export function isPriorGroupIdGap(ext: ObjectProperty): ext is PriorGroupIdGapProperty {
    return ext instanceof PriorGroupIdGapProperty
  }

  export function isPriorObjectIdGap(ext: ObjectProperty): ext is PriorObjectIdGapProperty {
    return ext instanceof PriorObjectIdGapProperty
  }

  export function isUnknown(ext: ObjectProperty): ext is UnknownObjectProperty {
    return ext instanceof UnknownObjectProperty
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  function roundtrip(exts: ObjectProperty[]): ObjectProperty[] {
    const buf = new ByteBuffer()
    ObjectProperty.serializeInto(exts, buf)
    return ObjectProperty.deserializeAll(buf.freeze())
  }

  describe('ObjectProperty', () => {
    test('empty deserializeAll returns []', () => {
      const buf = new FrozenByteBuffer(new Uint8Array())
      expect(ObjectProperty.deserializeAll(buf)).toEqual([])
    })

    test('PriorGroupIdGapProperty roundtrip', () => {
      const ext = new PriorGroupIdGapProperty(7n)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(PriorGroupIdGapProperty)
      expect((result as PriorGroupIdGapProperty).gap).toBe(7n)
    })

    test('PriorObjectIdGapProperty roundtrip', () => {
      const ext = new PriorObjectIdGapProperty(3n)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(PriorObjectIdGapProperty)
      expect((result as PriorObjectIdGapProperty).gap).toBe(3n)
    })

    test('ImmutablePropertiesObjectProperty roundtrip', () => {
      const inner = [KeyValuePair.tryNewVarInt(0x04, 100n)]
      const ext = new ImmutablePropertiesObjectProperty(inner)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(ImmutablePropertiesObjectProperty)
      expect((result as ImmutablePropertiesObjectProperty).properties.length).toBe(1)
    })

    test('ImmutablePropertiesObjectProperty throws on nested 0x0B', () => {
      const nested = new ImmutablePropertiesObjectProperty([])
      const innerKvp = nested.toKeyValuePair()
      const outer = new ByteBuffer()
      outer.putBytes(innerKvp.serialize().toUint8Array())
      const outerKvp = KeyValuePair.tryNewBytes(TrackPropertyType.ImmutableProperties, outer.toUint8Array())
      expect(() => ImmutablePropertiesObjectProperty.fromKeyValuePair(outerKvp)).toThrow(ProtocolViolationError)
    })

    test('deserializeRaw rejects a Mandatory Track Property, directly or nested', () => {
      const raw = (pairs: KeyValuePair[]) => ObjectProperty.deserializeRaw(serializeKvpList(pairs))
      const direct = KeyValuePair.tryNewVarInt(0x4000, 1n)
      const nested = new ImmutablePropertiesObjectProperty([
        KeyValuePair.tryNewBytes(0x7fff, new Uint8Array([1])),
      ]).toKeyValuePair()
      expect(() => raw([direct])).toThrow(ProtocolViolationError)
      expect(() => raw([nested])).toThrow(ProtocolViolationError)
      const outside = [KeyValuePair.tryNewVarInt(0x8000, 1n)]
      expect(raw(outside)).toEqual(outside)
      // A GREASE value inside the range is an ordinary unknown property.
      const grease = greaseValue(129)!
      expect(grease >= 0x4000n && grease <= 0x7fffn).toBe(true)
      const greased = [KeyValuePair.tryNewVarInt(grease, 1n)]
      expect(raw(greased)).toEqual(greased)
    })

    test('deserializeRaw turns a truncated block into a protocol violation', () => {
      const truncatedNested = KeyValuePair.tryNewBytes(TrackPropertyType.ImmutableProperties, new Uint8Array([0x02]))
      expect(() => ObjectProperty.deserializeRaw(serializeKvpList([truncatedNested]))).toThrow(ProtocolViolationError)
      expect(() => ObjectProperty.deserializeRaw(new FrozenByteBuffer(new Uint8Array([0x02])))).toThrow(
        ProtocolViolationError,
      )
    })

    test('UnknownObjectProperty pass-through roundtrip', () => {
      const kvp = KeyValuePair.tryNewVarInt(0x02, 42n)
      const ext = new UnknownObjectProperty(kvp)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(UnknownObjectProperty)
      expect((result as UnknownObjectProperty).kvp.typeValue).toBe(0x02n)
    })

    test('mixed list roundtrip', () => {
      // Order matches the canonical ascending-by-type wire order (delta-encoding requirement).
      const exts: ObjectProperty[] = [
        new ImmutablePropertiesObjectProperty([]),
        new PriorGroupIdGapProperty(5n),
        new PriorObjectIdGapProperty(2n),
      ]
      const result = roundtrip(exts)
      expect(result.length).toBe(3)
      expect(result[0]).toBeInstanceOf(ImmutablePropertiesObjectProperty)
      expect(result[1]).toBeInstanceOf(PriorGroupIdGapProperty)
      expect(result[2]).toBeInstanceOf(PriorObjectIdGapProperty)
    })

    test('ImmutableProperties independent of outer prevType', () => {
      // A low-type property precedes ImmutableProperties (0x0B) in the outer
      // list, so the outer prevType is nonzero by the time ImmutableProperties
      // is reached. The inner KVP list must restart its own delta state from
      // 0, independent of the outer list's running prevType.
      const inner = [
        KeyValuePair.tryNewVarInt(0x02, 7n),
        KeyValuePair.tryNewBytes(0x03, new TextEncoder().encode('data')),
      ]
      const exts: ObjectProperty[] = [
        new UnknownObjectProperty(KeyValuePair.tryNewVarInt(0x04, 99n)),
        new ImmutablePropertiesObjectProperty(inner),
      ]
      const result = roundtrip(exts)
      expect(result).toEqual(exts)
    })
  })
}
