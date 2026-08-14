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
import { KeyValuePair, deserializeKvpListUntilEmpty, serializeKvpList } from '../common/pair'
import { ProtocolViolationError } from '../error/error'
import { GroupOrder } from '../control/constant'
import { TrackPropertyType } from './constant'

export class DeliveryTimeoutProperty {
  static readonly TYPE = TrackPropertyType.DeliveryTimeout

  constructor(public readonly timeoutMs: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(DeliveryTimeoutProperty.TYPE, this.timeoutMs)
  }

  static fromKeyValuePair(pair: KeyValuePair): DeliveryTimeoutProperty | undefined {
    if (Number(pair.typeValue) !== DeliveryTimeoutProperty.TYPE || typeof pair.value !== 'bigint') return undefined
    if (pair.value === 0n) {
      throw new ProtocolViolationError(
        'DeliveryTimeoutProperty.fromKeyValuePair',
        'DELIVERY_TIMEOUT must be greater than 0',
      )
    }
    return new DeliveryTimeoutProperty(pair.value)
  }
}

export class MaxCacheDurationProperty {
  static readonly TYPE = TrackPropertyType.MaxCacheDuration

  constructor(public readonly durationMs: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MaxCacheDurationProperty.TYPE, this.durationMs)
  }

  static fromKeyValuePair(pair: KeyValuePair): MaxCacheDurationProperty | undefined {
    if (Number(pair.typeValue) !== MaxCacheDurationProperty.TYPE || typeof pair.value !== 'bigint') return undefined
    return new MaxCacheDurationProperty(pair.value)
  }
}

export class ImmutablePropertiesTrackProperty {
  static readonly TYPE = TrackPropertyType.ImmutableProperties

  constructor(public readonly properties: KeyValuePair[]) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewBytes(
      ImmutablePropertiesTrackProperty.TYPE,
      serializeKvpList(this.properties).toUint8Array(),
    )
  }

  static fromKeyValuePair(pair: KeyValuePair): ImmutablePropertiesTrackProperty | undefined {
    if (Number(pair.typeValue) !== ImmutablePropertiesTrackProperty.TYPE || !(pair.value instanceof Uint8Array))
      return undefined
    const buf = new FrozenByteBuffer(pair.value)
    const properties = deserializeKvpListUntilEmpty(buf)
    if (properties.some((inner) => Number(inner.typeValue) === TrackPropertyType.ImmutableProperties)) {
      throw new ProtocolViolationError(
        'ImmutablePropertiesTrackProperty.fromKeyValuePair',
        'ImmutableProperties must not contain nested ImmutableProperties (0x0B)',
      )
    }
    return new ImmutablePropertiesTrackProperty(properties)
  }
}

export class DefaultPublisherPriorityProperty {
  static readonly TYPE = TrackPropertyType.DefaultPublisherPriority

  constructor(public readonly priority: number) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(DefaultPublisherPriorityProperty.TYPE, this.priority)
  }

  static fromKeyValuePair(pair: KeyValuePair): DefaultPublisherPriorityProperty | undefined {
    if (Number(pair.typeValue) !== DefaultPublisherPriorityProperty.TYPE || typeof pair.value !== 'bigint')
      return undefined
    if (pair.value > 255n) {
      throw new ProtocolViolationError(
        'DefaultPublisherPriorityProperty.fromKeyValuePair',
        `Priority must be 0-255, got ${pair.value}`,
      )
    }
    return new DefaultPublisherPriorityProperty(Number(pair.value))
  }
}

export class DefaultPublisherGroupOrderProperty {
  static readonly TYPE = TrackPropertyType.DefaultPublisherGroupOrder

  constructor(public readonly order: GroupOrder) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(DefaultPublisherGroupOrderProperty.TYPE, this.order)
  }

  static fromKeyValuePair(pair: KeyValuePair): DefaultPublisherGroupOrderProperty | undefined {
    if (Number(pair.typeValue) !== DefaultPublisherGroupOrderProperty.TYPE || typeof pair.value !== 'bigint')
      return undefined
    if (pair.value !== 1n && pair.value !== 2n) {
      throw new ProtocolViolationError(
        'DefaultPublisherGroupOrderProperty.fromKeyValuePair',
        `GroupOrder must be Ascending(1) or Descending(2), got ${pair.value}`,
      )
    }
    return new DefaultPublisherGroupOrderProperty(Number(pair.value) as GroupOrder)
  }
}

export class DynamicGroupsProperty {
  static readonly TYPE = TrackPropertyType.DynamicGroups

  constructor(public readonly enabled: boolean) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(DynamicGroupsProperty.TYPE, this.enabled ? 1 : 0)
  }

  static fromKeyValuePair(pair: KeyValuePair): DynamicGroupsProperty | undefined {
    if (Number(pair.typeValue) !== DynamicGroupsProperty.TYPE || typeof pair.value !== 'bigint') return undefined
    if (pair.value > 1n) {
      throw new ProtocolViolationError(
        'DynamicGroupsProperty.fromKeyValuePair',
        `DynamicGroups must be 0 or 1, got ${pair.value}`,
      )
    }
    return new DynamicGroupsProperty(pair.value === 1n)
  }
}

export class UnknownTrackProperty {
  constructor(public readonly kvp: KeyValuePair) {}

  toKeyValuePair(): KeyValuePair {
    return this.kvp
  }
}

export type TrackProperty =
  | DeliveryTimeoutProperty
  | MaxCacheDurationProperty
  | ImmutablePropertiesTrackProperty
  | DefaultPublisherPriorityProperty
  | DefaultPublisherGroupOrderProperty
  | DynamicGroupsProperty
  | UnknownTrackProperty

export namespace TrackProperty {
  export function fromKeyValuePair(pair: KeyValuePair): TrackProperty {
    return (
      DeliveryTimeoutProperty.fromKeyValuePair(pair) ??
      MaxCacheDurationProperty.fromKeyValuePair(pair) ??
      ImmutablePropertiesTrackProperty.fromKeyValuePair(pair) ??
      DefaultPublisherPriorityProperty.fromKeyValuePair(pair) ??
      DefaultPublisherGroupOrderProperty.fromKeyValuePair(pair) ??
      DynamicGroupsProperty.fromKeyValuePair(pair) ??
      new UnknownTrackProperty(pair)
    )
  }

  export function toKeyValuePair(ext: TrackProperty): KeyValuePair {
    return ext.toKeyValuePair()
  }

  export function deserializeAll(buf: BaseByteBuffer): TrackProperty[] {
    return deserializeKvpListUntilEmpty(buf).map(fromKeyValuePair)
  }

  export function serializeInto(exts: TrackProperty[], payload: ByteBuffer): void {
    payload.putBytes(serializeKvpList(exts.map((ext) => ext.toKeyValuePair())).toUint8Array())
  }

  export function isDeliveryTimeout(ext: TrackProperty): ext is DeliveryTimeoutProperty {
    return ext instanceof DeliveryTimeoutProperty
  }

  export function isMaxCacheDuration(ext: TrackProperty): ext is MaxCacheDurationProperty {
    return ext instanceof MaxCacheDurationProperty
  }

  export function isImmutableProperties(ext: TrackProperty): ext is ImmutablePropertiesTrackProperty {
    return ext instanceof ImmutablePropertiesTrackProperty
  }

  export function isDefaultPublisherPriority(ext: TrackProperty): ext is DefaultPublisherPriorityProperty {
    return ext instanceof DefaultPublisherPriorityProperty
  }

  export function isDefaultPublisherGroupOrder(ext: TrackProperty): ext is DefaultPublisherGroupOrderProperty {
    return ext instanceof DefaultPublisherGroupOrderProperty
  }

  export function isDynamicGroups(ext: TrackProperty): ext is DynamicGroupsProperty {
    return ext instanceof DynamicGroupsProperty
  }

  export function isUnknown(ext: TrackProperty): ext is UnknownTrackProperty {
    return ext instanceof UnknownTrackProperty
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  function roundtrip(exts: TrackProperty[]): TrackProperty[] {
    const buf = new ByteBuffer()
    TrackProperty.serializeInto(exts, buf)
    return TrackProperty.deserializeAll(buf.freeze())
  }

  describe('TrackProperty', () => {
    test('empty deserializeAll returns []', () => {
      const buf = new FrozenByteBuffer(new Uint8Array())
      expect(TrackProperty.deserializeAll(buf)).toEqual([])
    })

    test('DeliveryTimeoutProperty roundtrip', () => {
      const ext = new DeliveryTimeoutProperty(5000n)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DeliveryTimeoutProperty)
      expect((result as DeliveryTimeoutProperty).timeoutMs).toBe(5000n)
    })

    test('DeliveryTimeoutProperty throws on zero', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackPropertyType.DeliveryTimeout, 0)
      expect(() => DeliveryTimeoutProperty.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('MaxCacheDurationProperty roundtrip', () => {
      const ext = new MaxCacheDurationProperty(12345n)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(MaxCacheDurationProperty)
      expect((result as MaxCacheDurationProperty).durationMs).toBe(12345n)
    })

    test('DefaultPublisherPriorityProperty roundtrip', () => {
      const ext = new DefaultPublisherPriorityProperty(128)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DefaultPublisherPriorityProperty)
      expect((result as DefaultPublisherPriorityProperty).priority).toBe(128)
    })

    test('DefaultPublisherPriorityProperty throws on priority > 255', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackPropertyType.DefaultPublisherPriority, 256)
      expect(() => DefaultPublisherPriorityProperty.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('DefaultPublisherGroupOrderProperty roundtrip ascending', () => {
      const ext = new DefaultPublisherGroupOrderProperty(GroupOrder.Ascending)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DefaultPublisherGroupOrderProperty)
      expect((result as DefaultPublisherGroupOrderProperty).order).toBe(GroupOrder.Ascending)
    })

    test('DefaultPublisherGroupOrderProperty roundtrip descending', () => {
      const ext = new DefaultPublisherGroupOrderProperty(GroupOrder.Descending)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DefaultPublisherGroupOrderProperty)
      expect((result as DefaultPublisherGroupOrderProperty).order).toBe(GroupOrder.Descending)
    })

    test('DefaultPublisherGroupOrderProperty throws on Original(0)', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackPropertyType.DefaultPublisherGroupOrder, 0)
      expect(() => DefaultPublisherGroupOrderProperty.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('DefaultPublisherGroupOrderProperty throws on invalid(3)', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackPropertyType.DefaultPublisherGroupOrder, 3)
      expect(() => DefaultPublisherGroupOrderProperty.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('DynamicGroupsProperty roundtrip enabled', () => {
      const ext = new DynamicGroupsProperty(true)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DynamicGroupsProperty)
      expect((result as DynamicGroupsProperty).enabled).toBe(true)
    })

    test('DynamicGroupsProperty roundtrip disabled', () => {
      const ext = new DynamicGroupsProperty(false)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DynamicGroupsProperty)
      expect((result as DynamicGroupsProperty).enabled).toBe(false)
    })

    test('DynamicGroupsProperty throws on value > 1', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackPropertyType.DynamicGroups, 2)
      expect(() => DynamicGroupsProperty.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('ImmutablePropertiesTrackProperty roundtrip', () => {
      const inner = [
        KeyValuePair.tryNewVarInt(0x04, 100n),
        KeyValuePair.tryNewBytes(0x0d, new TextEncoder().encode('meta')),
      ]
      const ext = new ImmutablePropertiesTrackProperty(inner)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(ImmutablePropertiesTrackProperty)
      const r = result as ImmutablePropertiesTrackProperty
      expect(r.properties.length).toBe(2)
      expect(r.properties[0]!.typeValue).toBe(0x04n)
      expect(r.properties[1]!.typeValue).toBe(0x0dn)
    })

    test('ImmutablePropertiesTrackProperty throws on nested 0x0B', () => {
      const nested = new ImmutablePropertiesTrackProperty([])
      const innerKvp = nested.toKeyValuePair()
      const outer = new ByteBuffer()
      outer.putBytes(innerKvp.serialize().toUint8Array())
      const outerKvp = KeyValuePair.tryNewBytes(TrackPropertyType.ImmutableProperties, outer.toUint8Array())
      expect(() => ImmutablePropertiesTrackProperty.fromKeyValuePair(outerKvp)).toThrow(ProtocolViolationError)
    })

    test('UnknownTrackProperty pass-through roundtrip', () => {
      const kvp = KeyValuePair.tryNewVarInt(0x3c, 99n)
      const ext = new UnknownTrackProperty(kvp)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(UnknownTrackProperty)
      expect((result as UnknownTrackProperty).kvp.typeValue).toBe(0x3cn)
    })

    test('mixed list roundtrip', () => {
      // Order matches the canonical ascending-by-type wire order (delta-encoding requirement).
      const exts: TrackProperty[] = [
        new DeliveryTimeoutProperty(1000n),
        new MaxCacheDurationProperty(500n),
        new DefaultPublisherPriorityProperty(42),
        new DynamicGroupsProperty(true),
      ]
      const result = roundtrip(exts)
      expect(result.length).toBe(4)
      expect(result[0]).toBeInstanceOf(DeliveryTimeoutProperty)
      expect(result[1]).toBeInstanceOf(MaxCacheDurationProperty)
      expect(result[2]).toBeInstanceOf(DefaultPublisherPriorityProperty)
      expect(result[3]).toBeInstanceOf(DynamicGroupsProperty)
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
      const exts: TrackProperty[] = [new DeliveryTimeoutProperty(100n), new ImmutablePropertiesTrackProperty(inner)]
      const result = roundtrip(exts)
      expect(result).toEqual(exts)
    })
  })
}
