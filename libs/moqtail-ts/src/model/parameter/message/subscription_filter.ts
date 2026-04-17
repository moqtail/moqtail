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
import { FilterType } from '../../control/constant'
import { Location } from '../../common'

/**
 * Length-prefixed Subscription Filter specifying what objects the subscriber wants.
 * If omitted from SUBSCRIBE or PUBLISH_OK, the subscription is unfiltered (LatestObject).
 * If omitted from REQUEST_UPDATE, the value is unchanged.
 */
export class SubscriptionFilter implements Parameter {
  static readonly TYPE = MessageParameterType.SubscriptionFilter

  constructor(
    public readonly filterType: FilterType,
    public readonly startLocation?: Location,
    public readonly endGroup?: bigint,
  ) {}

  toKeyValuePair(): KeyValuePair {
    const buf = new ByteBuffer()
    buf.putVI(this.filterType)
    if (this.filterType === FilterType.AbsoluteStart || this.filterType === FilterType.AbsoluteRange) {
      if (this.startLocation) {
        buf.putLocation(this.startLocation)
      }
    }
    if (this.filterType === FilterType.AbsoluteRange && this.endGroup != null) {
      buf.putVI(this.endGroup)
    }
    return KeyValuePair.tryNewBytes(SubscriptionFilter.TYPE, buf.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): SubscriptionFilter | undefined {
    if (Number(pair.typeValue) !== SubscriptionFilter.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const buf = new ByteBuffer()
    buf.putBytes(pair.value)
    const filterType = Number(buf.getVI()) as FilterType
    let startLocation: Location | undefined
    let endGroup: bigint | undefined
    if (filterType === FilterType.AbsoluteStart || filterType === FilterType.AbsoluteRange) {
      startLocation = buf.getLocation()
    }
    if (filterType === FilterType.AbsoluteRange) {
      endGroup = buf.getVI()
    }
    return new SubscriptionFilter(filterType, startLocation, endGroup)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SubscriptionFilter', () => {
    test('roundtrips LatestObject', () => {
      const orig = new SubscriptionFilter(FilterType.LatestObject)
      const pair = orig.toKeyValuePair()
      const parsed = SubscriptionFilter.fromKeyValuePair(pair)
      expect(parsed?.filterType).toBe(FilterType.LatestObject)
    })
    test('roundtrips AbsoluteStart', () => {
      const orig = new SubscriptionFilter(FilterType.AbsoluteStart, new Location(3n, 1n))
      const pair = orig.toKeyValuePair()
      const parsed = SubscriptionFilter.fromKeyValuePair(pair)
      expect(parsed?.filterType).toBe(FilterType.AbsoluteStart)
      expect(parsed?.startLocation?.group).toBe(3n)
      expect(parsed?.startLocation?.object).toBe(1n)
    })
    test('roundtrips AbsoluteRange', () => {
      const orig = new SubscriptionFilter(FilterType.AbsoluteRange, new Location(5n, 0n), 20n)
      const pair = orig.toKeyValuePair()
      const parsed = SubscriptionFilter.fromKeyValuePair(pair)
      expect(parsed?.filterType).toBe(FilterType.AbsoluteRange)
      expect(parsed?.startLocation?.group).toBe(5n)
      expect(parsed?.endGroup).toBe(20n)
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(MessageParameterType.DeliveryTimeout, 100n)
      expect(SubscriptionFilter.fromKeyValuePair(pair)).toBeUndefined()
    })
  })
}
