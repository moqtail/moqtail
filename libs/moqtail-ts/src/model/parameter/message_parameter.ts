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

import { ByteBuffer, isBytes, isVarInt, KeyValuePair } from '../common'
import { FilterType, GroupOrder } from '../control/constant'
import { Location } from '../common'

// --- Draft-16 Magic Numbers ---
// Even = VarInt, Odd = Bytes
export const PARAM_DELIVERY_TIMEOUT = 0x02n
export const PARAM_AUTHORIZATION_TOKEN = 0x03n
export const PARAM_EXPIRES = 0x08n
export const PARAM_LARGEST_OBJECT = 0x09n
export const PARAM_FORWARD = 0x10n
export const PARAM_SUBSCRIBER_PRIORITY = 0x20n
export const PARAM_SUBSCRIPTION_FILTER = 0x21n
export const PARAM_GROUP_ORDER = 0x22n
export const PARAM_NEW_GROUP_REQUEST = 0x32n

/**
 * @public
 * Strongly typed representation of Draft-16 Message Parameters.
 */
export class MessageParameters {
  public deliveryTimeout?: bigint | undefined
  public authorizationToken?: Uint8Array | undefined
  public expires?: bigint | undefined
  public largestObject?: Location | undefined

  // Protocol Defaults
  public forward: boolean = true
  public subscriberPriority: number = 128
  public groupOrder: GroupOrder = GroupOrder.Original

  // Filter components
  public filterType: FilterType = FilterType.LatestObject
  public startLocation?: Location | undefined
  public endGroup?: bigint | undefined

  public newGroupRequest?: bigint | undefined
  // Catch-all for unknown/custom extensions
  public unknownParameters: KeyValuePair[] = []

  constructor(init?: Partial<MessageParameters>) {
    if (init) {
      Object.assign(this, init)
    }
  }

  /**
   * Parses an array of KeyValuePairs into a strongly-typed MessageParameters struct.
   */
  static fromKeyValuePairs(pairs: KeyValuePair[]): MessageParameters {
    const params = new MessageParameters()

    for (const pair of pairs) {
      const type = pair.typeValue

      if (isVarInt(pair)) {
        const val = pair.value
        switch (type) {
          case PARAM_FORWARD:
            params.forward = val === 1n
            break
          case PARAM_SUBSCRIBER_PRIORITY:
            params.subscriberPriority = Number(val)
            break
          case PARAM_GROUP_ORDER:
            params.groupOrder = Number(val) as GroupOrder
            break
          case PARAM_DELIVERY_TIMEOUT:
            params.deliveryTimeout = val
            break
          case PARAM_EXPIRES:
            params.expires = val
            break
          case PARAM_NEW_GROUP_REQUEST:
            params.newGroupRequest = val
            break
          default:
            params.unknownParameters.push(pair)
        }
      } else if (isBytes(pair)) {
        const buf = new ByteBuffer()
        buf.putBytes(pair.value)

        switch (type) {
          case PARAM_AUTHORIZATION_TOKEN:
            params.authorizationToken = pair.value
            break
          case PARAM_LARGEST_OBJECT:
            try {
              params.largestObject = buf.getLocation()
            } catch (e) {
              // Ignore malformed location
            }
            break
          case PARAM_SUBSCRIPTION_FILTER:
            try {
              const filterTypeRaw = Number(buf.getVI()) as FilterType
              params.filterType = filterTypeRaw

              if (filterTypeRaw === FilterType.AbsoluteStart || filterTypeRaw === FilterType.AbsoluteRange) {
                params.startLocation = buf.getLocation()
              }
              if (filterTypeRaw === FilterType.AbsoluteRange) {
                params.endGroup = buf.getVI()
              }
            } catch (e) {
              // Ignore malformed filter
            }
            break
          default:
            params.unknownParameters.push(pair)
        }
      }
    }

    return params
  }

  /**
   * Serializes the explicit fields back into the Draft-16 KeyValuePair array.
   */
  intoKeyValuePairs(): KeyValuePair[] {
    const pairs: KeyValuePair[] = [...this.unknownParameters]

    // Only serialize non-default values to save wire space
    if (!this.forward) {
      pairs.push(KeyValuePair.tryNewVarInt(PARAM_FORWARD, 0n))
    }
    if (this.subscriberPriority !== 128) {
      pairs.push(KeyValuePair.tryNewVarInt(PARAM_SUBSCRIBER_PRIORITY, BigInt(this.subscriberPriority)))
    }
    if (this.groupOrder !== GroupOrder.Original) {
      pairs.push(KeyValuePair.tryNewVarInt(PARAM_GROUP_ORDER, BigInt(this.groupOrder)))
    }

    // Serialize Options
    if (this.deliveryTimeout != null) {
      pairs.push(KeyValuePair.tryNewVarInt(PARAM_DELIVERY_TIMEOUT, this.deliveryTimeout))
    }
    if (this.expires != null) {
      pairs.push(KeyValuePair.tryNewVarInt(PARAM_EXPIRES, this.expires))
    }
    if (this.newGroupRequest != null) {
      pairs.push(KeyValuePair.tryNewVarInt(PARAM_NEW_GROUP_REQUEST, this.newGroupRequest))
    }
    if (this.authorizationToken != null) {
      pairs.push(KeyValuePair.tryNewBytes(PARAM_AUTHORIZATION_TOKEN, this.authorizationToken))
    }

    // Serialize Largest Object
    if (this.largestObject) {
      const locBuf = new ByteBuffer()
      locBuf.putVI(this.largestObject.group)
      locBuf.putVI(this.largestObject.object)
      pairs.push(KeyValuePair.tryNewBytes(PARAM_LARGEST_OBJECT, locBuf.toUint8Array()))
    }

    // Serialize Subscription Filter
    if (this.filterType !== FilterType.LatestObject) {
      const filterBuf = new ByteBuffer()
      filterBuf.putVI(this.filterType)

      if (this.filterType === FilterType.AbsoluteStart || this.filterType === FilterType.AbsoluteRange) {
        if (this.startLocation) {
          filterBuf.putLocation(this.startLocation)
        }
      }
      if (this.filterType === FilterType.AbsoluteRange) {
        if (this.endGroup != null) {
          filterBuf.putVI(this.endGroup)
        }
      }
      pairs.push(KeyValuePair.tryNewBytes(PARAM_SUBSCRIPTION_FILTER, filterBuf.toUint8Array()))
    }

    return pairs
  }

  /**
   * Strategically applies updates based on an update message.
   * Per Draft-16: "If omitted from REQUEST_UPDATE/SUBSCRIBE_UPDATE, the value is unchanged."
   */
  applyUpdate(updates: KeyValuePair[]): void {
    for (const pair of updates) {
      const type = pair.typeValue

      if (isVarInt(pair)) {
        const val = pair.value
        switch (type) {
          case PARAM_FORWARD:
            this.forward = val === 1n
            break
          case PARAM_SUBSCRIBER_PRIORITY:
            this.subscriberPriority = Number(val)
            break
          case PARAM_GROUP_ORDER:
            this.groupOrder = Number(val) as GroupOrder
            break
          case PARAM_DELIVERY_TIMEOUT:
            this.deliveryTimeout = val
            break
          case PARAM_EXPIRES:
            this.expires = val
            break
          case PARAM_NEW_GROUP_REQUEST:
            this.newGroupRequest = val
            break
          default:
            this.updateUnknown(pair)
        }
      } else if (isBytes(pair)) {
        const buf = new ByteBuffer()
        buf.putBytes(pair.value)

        switch (type) {
          case PARAM_AUTHORIZATION_TOKEN:
            this.authorizationToken = pair.value
            break
          case PARAM_LARGEST_OBJECT:
            try {
              this.largestObject = buf.getLocation()
            } catch (e) {
              // Ignore
            }
            break
          case PARAM_SUBSCRIPTION_FILTER:
            try {
              const filterTypeRaw = Number(buf.getVI()) as FilterType
              this.filterType = filterTypeRaw
              // Reset
              this.startLocation = undefined
              this.endGroup = undefined

              if (filterTypeRaw === FilterType.AbsoluteStart || filterTypeRaw === FilterType.AbsoluteRange) {
                this.startLocation = buf.getLocation()
              }
              if (filterTypeRaw === FilterType.AbsoluteRange) {
                this.endGroup = buf.getVI()
              }
            } catch (e) {
              // Ignore
            }
            break
          default:
            this.updateUnknown(pair)
        }
      }
    }
  }

  private updateUnknown(pair: KeyValuePair): void {
    const existingIdx = this.unknownParameters.findIndex((p) => p.typeValue === pair.typeValue)
    if (existingIdx >= 0) {
      this.unknownParameters[existingIdx] = pair
    } else {
      this.unknownParameters.push(pair)
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MessageParameters', () => {
    test('roundtrips known and unknown parameters correctly', () => {
      const params = new MessageParameters({
        deliveryTimeout: 150n,
        subscriberPriority: 42,
        forward: false,
        filterType: FilterType.AbsoluteRange,
        startLocation: new Location(10n, 0n),
        endGroup: 20n,
        unknownParameters: [KeyValuePair.tryNewVarInt(998n, 1n)],
      })

      const kvps = params.intoKeyValuePairs()
      const parsed = MessageParameters.fromKeyValuePairs(kvps)

      expect(parsed.deliveryTimeout).toBe(150n)
      expect(parsed.subscriberPriority).toBe(42)
      expect(parsed.forward).toBe(false)
      expect(parsed.filterType).toBe(FilterType.AbsoluteRange)
      expect(parsed.startLocation?.group).toBe(10n)
      expect(parsed.endGroup).toBe(20n)
      expect(parsed.unknownParameters.length).toBe(1)
      expect(parsed.unknownParameters[0]!.typeValue).toBe(998n)
    })

    test('omits default values during serialization', () => {
      const params = new MessageParameters() // Completely default
      const kvps = params.intoKeyValuePairs()

      // Should serialize nothing since everything is default
      expect(kvps.length).toBe(0)
    })
  })
}
