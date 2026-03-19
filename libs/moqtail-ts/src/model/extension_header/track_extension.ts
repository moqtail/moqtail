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
import { GroupOrder } from '../control/constant'
import { TrackExtensionType } from './constant'

export class DeliveryTimeoutExtension {
  static readonly TYPE = TrackExtensionType.DeliveryTimeout

  constructor(public readonly timeoutMs: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(DeliveryTimeoutExtension.TYPE, this.timeoutMs)
  }

  static fromKeyValuePair(pair: KeyValuePair): DeliveryTimeoutExtension | undefined {
    if (Number(pair.typeValue) !== DeliveryTimeoutExtension.TYPE || typeof pair.value !== 'bigint') return undefined
    if (pair.value === 0n) {
      throw new ProtocolViolationError(
        'DeliveryTimeoutExtension.fromKeyValuePair',
        'DELIVERY_TIMEOUT must be greater than 0',
      )
    }
    return new DeliveryTimeoutExtension(pair.value)
  }
}

export class MaxCacheDurationExtension {
  static readonly TYPE = TrackExtensionType.MaxCacheDuration

  constructor(public readonly durationMs: bigint) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MaxCacheDurationExtension.TYPE, this.durationMs)
  }

  static fromKeyValuePair(pair: KeyValuePair): MaxCacheDurationExtension | undefined {
    if (Number(pair.typeValue) !== MaxCacheDurationExtension.TYPE || typeof pair.value !== 'bigint') return undefined
    return new MaxCacheDurationExtension(pair.value)
  }
}

export class ImmutableExtensionsExtension {
  static readonly TYPE = TrackExtensionType.ImmutableExtensions

  constructor(public readonly extensions: KeyValuePair[]) {}

  toKeyValuePair(): KeyValuePair {
    const inner = new ByteBuffer()
    for (const kvp of this.extensions) {
      inner.putBytes(kvp.serialize().toUint8Array())
    }
    return KeyValuePair.tryNewBytes(ImmutableExtensionsExtension.TYPE, inner.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): ImmutableExtensionsExtension | undefined {
    if (Number(pair.typeValue) !== ImmutableExtensionsExtension.TYPE || !(pair.value instanceof Uint8Array))
      return undefined
    const buf = new FrozenByteBuffer(pair.value)
    const extensions: KeyValuePair[] = []
    while (buf.remaining > 0) {
      const inner = KeyValuePair.deserialize(buf)
      if (Number(inner.typeValue) === TrackExtensionType.ImmutableExtensions) {
        throw new ProtocolViolationError(
          'ImmutableExtensionsExtension.fromKeyValuePair',
          'ImmutableExtensions must not contain nested ImmutableExtensions (0x0B)',
        )
      }
      extensions.push(inner)
    }
    return new ImmutableExtensionsExtension(extensions)
  }
}

export class DefaultPublisherPriorityExtension {
  static readonly TYPE = TrackExtensionType.DefaultPublisherPriority

  constructor(public readonly priority: number) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(DefaultPublisherPriorityExtension.TYPE, this.priority)
  }

  static fromKeyValuePair(pair: KeyValuePair): DefaultPublisherPriorityExtension | undefined {
    if (Number(pair.typeValue) !== DefaultPublisherPriorityExtension.TYPE || typeof pair.value !== 'bigint')
      return undefined
    if (pair.value > 255n) {
      throw new ProtocolViolationError(
        'DefaultPublisherPriorityExtension.fromKeyValuePair',
        `Priority must be 0-255, got ${pair.value}`,
      )
    }
    return new DefaultPublisherPriorityExtension(Number(pair.value))
  }
}

export class DefaultPublisherGroupOrderExtension {
  static readonly TYPE = TrackExtensionType.DefaultPublisherGroupOrder

  constructor(public readonly order: GroupOrder) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(DefaultPublisherGroupOrderExtension.TYPE, this.order)
  }

  static fromKeyValuePair(pair: KeyValuePair): DefaultPublisherGroupOrderExtension | undefined {
    if (Number(pair.typeValue) !== DefaultPublisherGroupOrderExtension.TYPE || typeof pair.value !== 'bigint')
      return undefined
    if (pair.value !== 1n && pair.value !== 2n) {
      throw new ProtocolViolationError(
        'DefaultPublisherGroupOrderExtension.fromKeyValuePair',
        `GroupOrder must be Ascending(1) or Descending(2), got ${pair.value}`,
      )
    }
    return new DefaultPublisherGroupOrderExtension(Number(pair.value) as GroupOrder)
  }
}

export class DynamicGroupsExtension {
  static readonly TYPE = TrackExtensionType.DynamicGroups

  constructor(public readonly enabled: boolean) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(DynamicGroupsExtension.TYPE, this.enabled ? 1 : 0)
  }

  static fromKeyValuePair(pair: KeyValuePair): DynamicGroupsExtension | undefined {
    if (Number(pair.typeValue) !== DynamicGroupsExtension.TYPE || typeof pair.value !== 'bigint') return undefined
    if (pair.value > 1n) {
      throw new ProtocolViolationError(
        'DynamicGroupsExtension.fromKeyValuePair',
        `DynamicGroups must be 0 or 1, got ${pair.value}`,
      )
    }
    return new DynamicGroupsExtension(pair.value === 1n)
  }
}

export class UnknownTrackExtension {
  constructor(public readonly kvp: KeyValuePair) {}

  toKeyValuePair(): KeyValuePair {
    return this.kvp
  }
}

export type TrackExtension =
  | DeliveryTimeoutExtension
  | MaxCacheDurationExtension
  | ImmutableExtensionsExtension
  | DefaultPublisherPriorityExtension
  | DefaultPublisherGroupOrderExtension
  | DynamicGroupsExtension
  | UnknownTrackExtension

export namespace TrackExtension {
  export function fromKeyValuePair(pair: KeyValuePair): TrackExtension {
    return (
      DeliveryTimeoutExtension.fromKeyValuePair(pair) ??
      MaxCacheDurationExtension.fromKeyValuePair(pair) ??
      ImmutableExtensionsExtension.fromKeyValuePair(pair) ??
      DefaultPublisherPriorityExtension.fromKeyValuePair(pair) ??
      DefaultPublisherGroupOrderExtension.fromKeyValuePair(pair) ??
      DynamicGroupsExtension.fromKeyValuePair(pair) ??
      new UnknownTrackExtension(pair)
    )
  }

  export function toKeyValuePair(ext: TrackExtension): KeyValuePair {
    return ext.toKeyValuePair()
  }

  export function deserializeAll(buf: BaseByteBuffer): TrackExtension[] {
    const result: TrackExtension[] = []
    while (buf.remaining > 0) {
      const kvp = KeyValuePair.deserialize(buf)
      result.push(fromKeyValuePair(kvp))
    }
    return result
  }

  export function serializeInto(exts: TrackExtension[], payload: ByteBuffer): void {
    for (const ext of exts) {
      payload.putBytes(ext.toKeyValuePair().serialize().toUint8Array())
    }
  }

  export function isDeliveryTimeout(ext: TrackExtension): ext is DeliveryTimeoutExtension {
    return ext instanceof DeliveryTimeoutExtension
  }

  export function isMaxCacheDuration(ext: TrackExtension): ext is MaxCacheDurationExtension {
    return ext instanceof MaxCacheDurationExtension
  }

  export function isImmutableExtensions(ext: TrackExtension): ext is ImmutableExtensionsExtension {
    return ext instanceof ImmutableExtensionsExtension
  }

  export function isDefaultPublisherPriority(ext: TrackExtension): ext is DefaultPublisherPriorityExtension {
    return ext instanceof DefaultPublisherPriorityExtension
  }

  export function isDefaultPublisherGroupOrder(ext: TrackExtension): ext is DefaultPublisherGroupOrderExtension {
    return ext instanceof DefaultPublisherGroupOrderExtension
  }

  export function isDynamicGroups(ext: TrackExtension): ext is DynamicGroupsExtension {
    return ext instanceof DynamicGroupsExtension
  }

  export function isUnknown(ext: TrackExtension): ext is UnknownTrackExtension {
    return ext instanceof UnknownTrackExtension
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  function roundtrip(exts: TrackExtension[]): TrackExtension[] {
    const buf = new ByteBuffer()
    TrackExtension.serializeInto(exts, buf)
    return TrackExtension.deserializeAll(buf.freeze())
  }

  describe('TrackExtension', () => {
    test('empty deserializeAll returns []', () => {
      const buf = new FrozenByteBuffer(new Uint8Array())
      expect(TrackExtension.deserializeAll(buf)).toEqual([])
    })

    test('DeliveryTimeoutExtension roundtrip', () => {
      const ext = new DeliveryTimeoutExtension(5000n)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DeliveryTimeoutExtension)
      expect((result as DeliveryTimeoutExtension).timeoutMs).toBe(5000n)
    })

    test('DeliveryTimeoutExtension throws on zero', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackExtensionType.DeliveryTimeout, 0)
      expect(() => DeliveryTimeoutExtension.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('MaxCacheDurationExtension roundtrip', () => {
      const ext = new MaxCacheDurationExtension(12345n)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(MaxCacheDurationExtension)
      expect((result as MaxCacheDurationExtension).durationMs).toBe(12345n)
    })

    test('DefaultPublisherPriorityExtension roundtrip', () => {
      const ext = new DefaultPublisherPriorityExtension(128)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DefaultPublisherPriorityExtension)
      expect((result as DefaultPublisherPriorityExtension).priority).toBe(128)
    })

    test('DefaultPublisherPriorityExtension throws on priority > 255', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackExtensionType.DefaultPublisherPriority, 256)
      expect(() => DefaultPublisherPriorityExtension.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('DefaultPublisherGroupOrderExtension roundtrip ascending', () => {
      const ext = new DefaultPublisherGroupOrderExtension(GroupOrder.Ascending)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DefaultPublisherGroupOrderExtension)
      expect((result as DefaultPublisherGroupOrderExtension).order).toBe(GroupOrder.Ascending)
    })

    test('DefaultPublisherGroupOrderExtension roundtrip descending', () => {
      const ext = new DefaultPublisherGroupOrderExtension(GroupOrder.Descending)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DefaultPublisherGroupOrderExtension)
      expect((result as DefaultPublisherGroupOrderExtension).order).toBe(GroupOrder.Descending)
    })

    test('DefaultPublisherGroupOrderExtension throws on Original(0)', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackExtensionType.DefaultPublisherGroupOrder, 0)
      expect(() => DefaultPublisherGroupOrderExtension.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('DefaultPublisherGroupOrderExtension throws on invalid(3)', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackExtensionType.DefaultPublisherGroupOrder, 3)
      expect(() => DefaultPublisherGroupOrderExtension.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('DynamicGroupsExtension roundtrip enabled', () => {
      const ext = new DynamicGroupsExtension(true)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DynamicGroupsExtension)
      expect((result as DynamicGroupsExtension).enabled).toBe(true)
    })

    test('DynamicGroupsExtension roundtrip disabled', () => {
      const ext = new DynamicGroupsExtension(false)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(DynamicGroupsExtension)
      expect((result as DynamicGroupsExtension).enabled).toBe(false)
    })

    test('DynamicGroupsExtension throws on value > 1', () => {
      const kvp = KeyValuePair.tryNewVarInt(TrackExtensionType.DynamicGroups, 2)
      expect(() => DynamicGroupsExtension.fromKeyValuePair(kvp)).toThrow(ProtocolViolationError)
    })

    test('ImmutableExtensionsExtension roundtrip', () => {
      const inner = [
        KeyValuePair.tryNewVarInt(0x04, 100n),
        KeyValuePair.tryNewBytes(0x0d, new TextEncoder().encode('meta')),
      ]
      const ext = new ImmutableExtensionsExtension(inner)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(ImmutableExtensionsExtension)
      const r = result as ImmutableExtensionsExtension
      expect(r.extensions.length).toBe(2)
      expect(r.extensions[0]!.typeValue).toBe(0x04n)
      expect(r.extensions[1]!.typeValue).toBe(0x0dn)
    })

    test('ImmutableExtensionsExtension throws on nested 0x0B', () => {
      const nested = new ImmutableExtensionsExtension([])
      const innerKvp = nested.toKeyValuePair()
      const outer = new ByteBuffer()
      outer.putBytes(innerKvp.serialize().toUint8Array())
      const outerKvp = KeyValuePair.tryNewBytes(TrackExtensionType.ImmutableExtensions, outer.toUint8Array())
      expect(() => ImmutableExtensionsExtension.fromKeyValuePair(outerKvp)).toThrow(ProtocolViolationError)
    })

    test('UnknownTrackExtension pass-through roundtrip', () => {
      const kvp = KeyValuePair.tryNewVarInt(0x3c, 99n)
      const ext = new UnknownTrackExtension(kvp)
      const [result] = roundtrip([ext])
      expect(result).toBeInstanceOf(UnknownTrackExtension)
      expect((result as UnknownTrackExtension).kvp.typeValue).toBe(0x3cn)
    })

    test('mixed list roundtrip', () => {
      const exts: TrackExtension[] = [
        new DeliveryTimeoutExtension(1000n),
        new MaxCacheDurationExtension(500n),
        new DynamicGroupsExtension(true),
        new DefaultPublisherPriorityExtension(42),
      ]
      const result = roundtrip(exts)
      expect(result.length).toBe(4)
      expect(result[0]).toBeInstanceOf(DeliveryTimeoutExtension)
      expect(result[1]).toBeInstanceOf(MaxCacheDurationExtension)
      expect(result[2]).toBeInstanceOf(DynamicGroupsExtension)
      expect(result[3]).toBeInstanceOf(DefaultPublisherPriorityExtension)
    })
  })
}
