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

import { KeyValuePair } from '../common/pair'
import { CastingError, ProtocolViolationError } from '../error/error'
import { ExtensionHeaders } from '../extension_header'
import { ObjectForwardingPreference, ObjectStatus, SubgroupHeaderType } from './constant'
import { DatagramObject } from './datagram_object'
import { DatagramStatus } from './datagram_status'
import { FetchObject } from './fetch_object'
import { FullTrackName } from './full_track_name'
import { SubgroupObject } from './subgroup_object'
import { Location } from '../common/location'

export class MoqtObject {
  public readonly location: Location
  public readonly subgroupId: bigint | null

  private constructor(
    public readonly fullTrackName: FullTrackName,
    location: Location,
    public readonly publisherPriority: number,
    public readonly objectForwardingPreference: ObjectForwardingPreference,
    subgroupId: bigint | number | null,
    public readonly objectStatus: ObjectStatus,
    public readonly extensionHeaders: KeyValuePair[] | null,
    public readonly payload: Uint8Array | null,
  ) {
    this.location = location
    this.subgroupId = subgroupId !== null ? BigInt(subgroupId) : null
  }

  get groupId(): bigint {
    return this.location.group
  }
  get objectId(): bigint {
    return this.location.object
  }

  getSubgroupHeaderType(containsEnd: boolean): SubgroupHeaderType {
    const hasExtensions = !!this.extensionHeaders && this.extensionHeaders.length > 0
    const hasSubgroupId = this.subgroupId !== null
    const isSubgroupIdZero = this.subgroupId === 0n

    if (containsEnd) {
      if (hasSubgroupId) {
        return hasExtensions ? SubgroupHeaderType.Type0x1D : SubgroupHeaderType.Type0x1C
      } else if (isSubgroupIdZero) {
        return hasExtensions ? SubgroupHeaderType.Type0x19 : SubgroupHeaderType.Type0x18
      } else {
        return hasExtensions ? SubgroupHeaderType.Type0x1B : SubgroupHeaderType.Type0x1A
      }
    } else {
      if (hasSubgroupId) {
        return hasExtensions ? SubgroupHeaderType.Type0x15 : SubgroupHeaderType.Type0x14
      } else if (isSubgroupIdZero) {
        return hasExtensions ? SubgroupHeaderType.Type0x11 : SubgroupHeaderType.Type0x10
      } else {
        return hasExtensions ? SubgroupHeaderType.Type0x13 : SubgroupHeaderType.Type0x12
      }
    }
  }

  isDatagram(): boolean {
    return this.objectForwardingPreference === ObjectForwardingPreference.Datagram
  }
  isSubgroup(): boolean {
    return this.objectForwardingPreference === ObjectForwardingPreference.Subgroup
  }
  isEndOfGroup(): boolean {
    return this.objectStatus === ObjectStatus.EndOfGroup
  }
  isEndOfTrack(): boolean {
    return this.objectStatus === ObjectStatus.EndOfTrack
  }
  doesNotExist(): boolean {
    return this.objectStatus === ObjectStatus.DoesNotExist
  }
  hasPayload(): boolean {
    return this.payload !== null
  }
  hasStatus(): boolean {
    return this.objectStatus !== ObjectStatus.Normal
  }

  static newWithPayload(
    fullTrackName: FullTrackName,
    location: Location,
    publisherPriority: number,
    objectForwardingPreference: ObjectForwardingPreference,
    subgroupId: bigint | number | null,
    extensionHeaders: KeyValuePair[] | null,
    payload: Uint8Array,
  ): MoqtObject {
    return new MoqtObject(
      fullTrackName,
      location,
      publisherPriority,
      objectForwardingPreference,
      subgroupId,
      ObjectStatus.Normal,
      extensionHeaders,
      payload,
    )
  }

  static newWithStatus(
    fullTrackName: FullTrackName,
    location: Location,
    publisherPriority: number,
    objectForwardingPreference: ObjectForwardingPreference,
    subgroupId: bigint | number | null,
    extensionHeaders: KeyValuePair[] | null,
    objectStatus: ObjectStatus,
  ): MoqtObject {
    return new MoqtObject(
      fullTrackName,
      location,
      publisherPriority,
      objectForwardingPreference,
      subgroupId,
      objectStatus,
      extensionHeaders,
      null,
    )
  }

  static fromDatagramObject(datagramObject: DatagramObject, fullTrackName: FullTrackName): MoqtObject {
    return new MoqtObject(
      fullTrackName,
      new Location(datagramObject.groupId, datagramObject.objectId),
      datagramObject.publisherPriority,
      ObjectForwardingPreference.Datagram,
      null,
      ObjectStatus.Normal,
      datagramObject.extensionHeaders,
      datagramObject.payload,
    )
  }

  static fromDatagramStatus(datagramStatus: DatagramStatus, fullTrackName: FullTrackName): MoqtObject {
    return new MoqtObject(
      fullTrackName,
      datagramStatus.location,
      datagramStatus.publisherPriority,
      ObjectForwardingPreference.Datagram,
      null,
      datagramStatus.objectStatus,
      datagramStatus.extensionHeaders,
      null,
    )
  }

  static fromFetchObject(fetchObject: FetchObject, fullTrackName: FullTrackName): MoqtObject {
    return new MoqtObject(
      fullTrackName,
      fetchObject.location,
      fetchObject.publisherPriority,
      ObjectForwardingPreference.Subgroup,
      fetchObject.subgroupId,
      fetchObject.objectStatus || ObjectStatus.Normal,
      fetchObject.extensionHeaders,
      fetchObject.payload,
    )
  }

  static fromSubgroupObject(
    subgroupObject: SubgroupObject,
    groupId: bigint | number,
    publisherPriority: number,
    subgroupId: bigint | number,
    fullTrackName: FullTrackName,
  ): MoqtObject {
    return new MoqtObject(
      fullTrackName,
      new Location(groupId, subgroupObject.objectId),
      publisherPriority,
      ObjectForwardingPreference.Subgroup,
      subgroupId,
      subgroupObject.objectStatus || ObjectStatus.Normal,
      subgroupObject.extensionHeaders,
      subgroupObject.payload,
    )
  }
  tryIntoDatagramObject(trackAlias: bigint | number): DatagramObject {
    if (this.objectForwardingPreference !== ObjectForwardingPreference.Datagram) {
      throw new CastingError(
        'MoqtObject.tryIntoDatagramObject',
        'MoqtObject',
        'DatagramObject',
        'Object Forwarding Preference must be Datagram',
      )
    }
    if (this.objectStatus !== ObjectStatus.Normal || !this.payload) {
      throw new ProtocolViolationError(
        'MoqtObject.tryIntoDatagramObject',
        'Object must have Normal status and payload for DatagramObject conversion',
      )
    }

    const alias = BigInt(trackAlias)
    if (this.extensionHeaders && this.extensionHeaders.length > 0) {
      return DatagramObject.newWithExtensions(
        alias,
        this.groupId,
        this.objectId,
        this.publisherPriority,
        this.extensionHeaders,
        this.payload!,
      )
    } else {
      return DatagramObject.newWithoutExtensions(
        alias,
        this.groupId,
        this.objectId,
        this.publisherPriority,
        this.payload!,
      )
    }
  }
  tryIntoDatagramStatus(trackAlias: bigint | number): DatagramStatus {
    if (this.objectForwardingPreference !== ObjectForwardingPreference.Datagram) {
      throw new CastingError(
        'MoqtObject.tryIntoDatagramStatus',
        'MoqtObject',
        'DatagramStatus',
        'Object Forwarding Preference must be Datagram',
      )
    }
    if (this.objectStatus === ObjectStatus.Normal) {
      throw new ProtocolViolationError(
        'MoqtObject.tryIntoDatagramStatus',
        'Object Status must not be Normal for DatagramStatus conversion',
      )
    }

    const alias = BigInt(trackAlias)
    if (this.extensionHeaders && this.extensionHeaders.length > 0) {
      return DatagramStatus.withExtensions(
        alias,
        this.location,
        this.publisherPriority,
        this.extensionHeaders,
        this.objectStatus,
      )
    } else {
      return DatagramStatus.newWithoutExtensions(alias, this.location, this.publisherPriority, this.objectStatus)
    }
  }
  tryIntoFetchObject(): FetchObject {
    if (this.objectForwardingPreference !== ObjectForwardingPreference.Subgroup) {
      throw new CastingError(
        'MoqtObject.tryIntoFetchObject',
        'MoqtObject',
        'FetchObject',
        'Object Forwarding Preference must be Subgroup',
      )
    }
    if (this.subgroupId === null) {
      throw new ProtocolViolationError(
        'MoqtObject.tryIntoFetchObject',
        'Subgroup ID is required for Subgroup forwarding preference',
      )
    }

    if (this.objectStatus === ObjectStatus.Normal && this.payload) {
      return FetchObject.newWithPayload(
        this.groupId,
        this.subgroupId!,
        this.objectId,
        this.publisherPriority,
        this.extensionHeaders,
        this.payload,
      )
    } else {
      return FetchObject.newWithStatus(
        this.groupId,
        this.subgroupId!,
        this.objectId,
        this.publisherPriority,
        this.extensionHeaders,
        this.objectStatus,
      )
    }
  }
  tryIntoSubgroupObject(): SubgroupObject {
    if (this.objectForwardingPreference !== ObjectForwardingPreference.Subgroup) {
      throw new CastingError(
        'MoqtObject.tryIntoSubgroupObject',
        'MoqtObject',
        'SubgroupObject',
        'Object Forwarding Preference must be Subgroup',
      )
    }
    if (this.objectStatus === ObjectStatus.Normal && this.payload) {
      return SubgroupObject.newWithPayload(this.location.object, this.extensionHeaders, this.payload)
    } else {
      return SubgroupObject.newWithStatus(this.location.object, this.extensionHeaders, this.objectStatus)
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MoqtObject', () => {
    test('create object with payload', () => {
      const payload = new TextEncoder().encode('test payload')
      const extensionHeaders = new ExtensionHeaders().addAudioLevel(100).addCaptureTimestamp(0).build()
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track1')
      const location = new Location(100n, 10n)
      const obj = MoqtObject.newWithPayload(
        fullTrackName,
        location,
        128,
        ObjectForwardingPreference.Subgroup,
        5n,
        extensionHeaders,
        payload,
      )

      expect(obj.location.equals(location)).toBe(true)
      expect(obj.groupId).toBe(100n)
      expect(obj.objectId).toBe(10n)
      expect(obj.publisherPriority).toBe(128)
      expect(obj.objectForwardingPreference).toBe(ObjectForwardingPreference.Subgroup)
      expect(obj.subgroupId).toBe(5n)
      expect(obj.objectStatus).toBe(ObjectStatus.Normal)
      expect(obj.extensionHeaders).toEqual(extensionHeaders)
      expect(obj.payload).toEqual(payload)
      expect(obj.hasPayload()).toBe(true)
      expect(obj.hasStatus()).toBe(false)
    })

    test('create object with status', () => {
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track2')
      const location = new Location(200n, 20n)
      const obj = MoqtObject.newWithStatus(
        fullTrackName,
        location,
        64,
        ObjectForwardingPreference.Datagram,
        null,
        null,
        ObjectStatus.EndOfGroup,
      )

      expect(obj.location.equals(location)).toBe(true)
      expect(obj.groupId).toBe(200n)
      expect(obj.objectId).toBe(20n)
      expect(obj.publisherPriority).toBe(64)
      expect(obj.objectForwardingPreference).toBe(ObjectForwardingPreference.Datagram)
      expect(obj.subgroupId).toBe(null)
      expect(obj.objectStatus).toBe(ObjectStatus.EndOfGroup)
      expect(obj.extensionHeaders).toBe(null)
      expect(obj.payload).toBe(null)
      expect(obj.hasPayload()).toBe(false)
      expect(obj.hasStatus()).toBe(true)
    })

    test('convert from/to DatagramObject', () => {
      const payload = new TextEncoder().encode('datagram payload')
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track3')
      const datagramObj = DatagramObject.newWithoutExtensions(42n, 100n, 10n, 128, payload)

      const moqtObj = MoqtObject.fromDatagramObject(datagramObj, fullTrackName)
      expect(moqtObj.objectForwardingPreference).toBe(ObjectForwardingPreference.Datagram)
      expect(moqtObj.subgroupId).toBe(null)

      const backToDatagram = moqtObj.tryIntoDatagramObject(42n)
      expect(backToDatagram.trackAlias).toBe(42n)
      expect(backToDatagram.groupId).toBe(100n)
      expect(backToDatagram.objectId).toBe(10n)
      expect(backToDatagram.payload).toEqual(payload)
    })

    test('convert from/to FetchObject', () => {
      const payload = new TextEncoder().encode('fetch payload')
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track4')
      const fetchObj = FetchObject.newWithPayload(100n, 5n, 10n, 128, null, payload)

      const moqtObj = MoqtObject.fromFetchObject(fetchObj, fullTrackName)
      expect(moqtObj.objectForwardingPreference).toBe(ObjectForwardingPreference.Subgroup)
      expect(moqtObj.subgroupId).toBe(5n)
      expect(moqtObj.location.equals(fetchObj.location)).toBe(true)

      const backToFetch = moqtObj.tryIntoFetchObject()
      expect(backToFetch.groupId).toBe(100n)
      expect(backToFetch.subgroupId).toBe(5n)
      expect(backToFetch.objectId).toBe(10n)
      expect(backToFetch.payload).toEqual(payload)
    })

    test('utility functions', () => {
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track5')
      const location1 = new Location(100n, 10n)
      const datagramObj = MoqtObject.newWithStatus(
        fullTrackName,
        location1,
        128,
        ObjectForwardingPreference.Datagram,
        null,
        null,
        ObjectStatus.EndOfTrack,
      )
      const location2 = new Location(100n, 10n)
      const subgroupObj = MoqtObject.newWithPayload(
        fullTrackName,
        location2,
        128,
        ObjectForwardingPreference.Subgroup,
        5n,
        null,
        new Uint8Array(),
      )

      expect(datagramObj.isDatagram()).toBe(true)
      expect(datagramObj.isSubgroup()).toBe(false)
      expect(subgroupObj.isDatagram()).toBe(false)
      expect(subgroupObj.isSubgroup()).toBe(true)

      expect(datagramObj.isEndOfTrack()).toBe(true)
      expect(datagramObj.isEndOfGroup()).toBe(false)
    })
  })
}
