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

import { KeyValuePair } from '@/model/common/pair'

/**
 * @public
 * A property class read by {@link findProperty}: its `TYPE` and `fromKeyValuePair`.
 * Every property class in the model already is one.
 */
export interface PropertyReader<T> {
  readonly TYPE: number | bigint
  fromKeyValuePair(pair: KeyValuePair): T | undefined
}

/**
 * @public
 * Finds the first property of `reader`'s type in a track or object property list and
 * returns it typed, or `undefined`. Unknown properties keep their raw key-value pair, so
 * a type the model doesn't know is still found.
 *
 * @example
 * ```ts
 * const timeout = findProperty(subscribeOk.trackProperties, ObjectDeliveryTimeoutProperty)
 * ```
 */
export function findProperty<T>(
  properties: readonly (KeyValuePair | { toKeyValuePair(): KeyValuePair })[] | null | undefined,
  reader: PropertyReader<T>,
): T | undefined {
  const type = BigInt(reader.TYPE)
  for (const property of properties ?? []) {
    const pair = property instanceof KeyValuePair ? property : property.toKeyValuePair()
    if (pair.typeValue === type) return reader.fromKeyValuePair(pair)
  }
  return undefined
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { MaxCacheDurationProperty, UnknownTrackProperty } = await import('./track_property')
  const { VideoConfig } = await import('./video_config')

  const CustomProperty = {
    TYPE: 0x3e,
    fromKeyValuePair: (pair: KeyValuePair) => (typeof pair.value === 'bigint' ? pair.value : undefined),
  }

  describe('findProperty', () => {
    test('reads a typed property out of a track property list, known or not', () => {
      const trackProperties = [
        new MaxCacheDurationProperty(1000n),
        new UnknownTrackProperty(KeyValuePair.tryNewVarInt(CustomProperty.TYPE, 7)),
      ]
      expect(findProperty(trackProperties, MaxCacheDurationProperty)?.durationMs).toBe(1000n)
      expect(findProperty(trackProperties, CustomProperty)).toBe(7n)
    })

    test('reads a typed property out of an object property list', () => {
      const properties = [new VideoConfig(new Uint8Array([1, 2])).toKeyValuePair()]
      expect(findProperty(properties, VideoConfig)?.config).toEqual(new Uint8Array([1, 2]))
      expect(findProperty(properties, MaxCacheDurationProperty)).toBeUndefined()
      expect(findProperty(null, VideoConfig)).toBeUndefined()
    })
  })
}
