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
import { ObjectDatagramType, ObjectForwardingPreference, ObjectStatus, SubgroupHeaderType } from './constant'
import { Datagram } from './datagram'
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

  getSubgroupHeaderType(containsEnd: boolean, useDefaultPriority: boolean = false): SubgroupHeaderType {
    const hasExtensions = !!this.extensionHeaders && this.extensionHeaders.length > 0

    // Determine SUBGROUP_ID_MODE (bits 1-2):
    // 0b00 (0): Subgroup ID = 0 (absent from header)
    // 0b01 (1): Subgroup ID = first Object ID (absent from header)
    // 0b10 (2): Subgroup ID = explicit (present in header)
    let subgroupIdMode: 0 | 1 | 2
    if (this.subgroupId === null) {
      subgroupIdMode = 1 // first-object-ID mode
    } else if (this.subgroupId === 0n) {
      subgroupIdMode = 0 // zero mode
    } else {
      subgroupIdMode = 2 // explicit mode
    }

    return SubgroupHeaderType.fromProperties(hasExtensions, subgroupIdMode, containsEnd, useDefaultPriority)
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

  /**
   * Create a MoqtObject from a Datagram (Draft-16).
   * @param datagram - The received Datagram.
   * @param fullTrackName - The resolved full track name.
   * @param defaultPriority - The default publisher priority from the control message (used when DEFAULT_PRIORITY bit is set).
   */
  static fromDatagram(datagram: Datagram, fullTrackName: FullTrackName, defaultPriority: number = 0): MoqtObject {
    const priority = datagram.publisherPriority ?? defaultPriority
    if (ObjectDatagramType.isStatus(datagram.type)) {
      return new MoqtObject(
        fullTrackName,
        datagram.location,
        priority,
        ObjectForwardingPreference.Datagram,
        null,
        datagram.objectStatus!,
        datagram.extensionHeaders,
        null,
      )
    } else {
      return new MoqtObject(
        fullTrackName,
        datagram.location,
        priority,
        ObjectForwardingPreference.Datagram,
        null,
        ObjectStatus.Normal,
        datagram.extensionHeaders,
        datagram.payload,
      )
    }
  }

  /**
   * Returns the endOfGroup flag from the source Datagram.
   * This is separate from ObjectStatus.EndOfGroup - the flag indicates
   * this is the last object in the group even with Normal status.
   */
  static isDatagramEndOfGroup(datagram: Datagram): boolean {
    return datagram.endOfGroup
  }

  static fromFetchObject(fetchObject: FetchObject, fullTrackName: FullTrackName): MoqtObject {
    if (fetchObject.kind !== 'object') {
      throw new ProtocolViolationError(
        'MoqtObject.fromFetchObject',
        'EndOfRange markers cannot be converted to MoqtObject',
      )
    }
    const subgroupId =
      fetchObject.forwardingPreference === ObjectForwardingPreference.Subgroup ? fetchObject.subgroupId : null
    return new MoqtObject(
      fullTrackName,
      fetchObject.location,
      fetchObject.publisherPriority,
      fetchObject.forwardingPreference,
      subgroupId,
      ObjectStatus.Normal,
      fetchObject.extensionHeaders,
      fetchObject.payload && fetchObject.payload.length > 0 ? fetchObject.payload : null,
    )
  }

  static fromSubgroupObject(
    subgroupObject: SubgroupObject,
    groupId: bigint | number,
    publisherPriority: number | undefined,
    subgroupId: bigint | number | null,
    fullTrackName: FullTrackName,
  ): MoqtObject {
    return new MoqtObject(
      fullTrackName,
      new Location(groupId, subgroupObject.objectId),
      publisherPriority ?? 0,
      ObjectForwardingPreference.Subgroup,
      subgroupId,
      subgroupObject.objectStatus || ObjectStatus.Normal,
      subgroupObject.extensionHeaders,
      subgroupObject.payload,
    )
  }
  /**
   * Convert to Datagram for wire transmission (Draft-16).
   * Automatically creates a payload or status Datagram based on the object's state.
   * @param trackAlias - The track alias to use.
   * @param endOfGroup - Whether this is the last object in the group (only for payload datagrams).
   * @param defaultPriority - If provided and matches this object's priority, the DEFAULT_PRIORITY bit is set.
   */
  tryIntoDatagram(trackAlias: bigint | number, endOfGroup: boolean = false, defaultPriority?: number): Datagram {
    if (this.objectForwardingPreference !== ObjectForwardingPreference.Datagram) {
      throw new CastingError(
        'MoqtObject.tryIntoDatagram',
        'MoqtObject',
        'Datagram',
        'Object Forwarding Preference must be Datagram',
      )
    }

    const alias = BigInt(trackAlias)
    const priority =
      defaultPriority !== undefined && defaultPriority === this.publisherPriority ? null : this.publisherPriority

    if (this.hasStatus()) {
      return Datagram.newStatus(alias, this.groupId, this.objectId, priority, this.extensionHeaders, this.objectStatus)
    } else {
      if (!this.payload) {
        throw new ProtocolViolationError(
          'MoqtObject.tryIntoDatagram',
          'Object must have payload for payload Datagram conversion',
        )
      }
      return Datagram.newPayload(
        alias,
        this.groupId,
        this.objectId,
        priority,
        this.extensionHeaders,
        this.payload,
        endOfGroup,
      )
    }
  }
  tryIntoFetchObject(): FetchObject {
    // Draft-16 §10.4.4 FETCH objects carry no status; non-Normal status must be
    // surfaced as an EndOfRange marker by the caller instead.
    if (this.objectStatus !== ObjectStatus.Normal) {
      throw new CastingError(
        'MoqtObject.tryIntoFetchObject',
        'MoqtObject',
        'FetchObject',
        'non-Normal status cannot be serialized as a fetch payload; use EndOfRange instead',
      )
    }
    let subgroupId: bigint | number
    if (this.objectForwardingPreference === ObjectForwardingPreference.Subgroup) {
      if (this.subgroupId === null) {
        throw new ProtocolViolationError(
          'MoqtObject.tryIntoFetchObject',
          'Subgroup ID is required for Subgroup forwarding preference',
        )
      }
      subgroupId = this.subgroupId
    } else {
      if (this.subgroupId !== null) {
        throw new ProtocolViolationError(
          'MoqtObject.tryIntoFetchObject',
          'Subgroup ID must not be set for Datagram forwarding preference',
        )
      }
      subgroupId = this.objectId
    }
    return FetchObject.newObject(
      this.groupId,
      subgroupId,
      this.objectId,
      this.publisherPriority,
      this.objectForwardingPreference,
      this.extensionHeaders,
      this.payload ?? new Uint8Array(0),
    )
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

    test('convert from/to Datagram payload (Draft-16)', () => {
      const payload = new TextEncoder().encode('datagram payload')
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track3')
      const datagram = Datagram.newPayload(42n, 100n, 10n, 128, null, payload, false)

      const moqtObj = MoqtObject.fromDatagram(datagram, fullTrackName)
      expect(moqtObj.objectForwardingPreference).toBe(ObjectForwardingPreference.Datagram)
      expect(moqtObj.subgroupId).toBe(null)
      expect(MoqtObject.isDatagramEndOfGroup(datagram)).toBe(false)

      const backToDatagram = moqtObj.tryIntoDatagram(42n, false)
      expect(backToDatagram.trackAlias).toBe(42n)
      expect(backToDatagram.groupId).toBe(100n)
      expect(backToDatagram.objectId).toBe(10n)
      expect(backToDatagram.endOfGroup).toBe(false)
      expect(backToDatagram.payload).toEqual(payload)
    })

    test('convert from/to Datagram with endOfGroup (Draft-16)', () => {
      const payload = new TextEncoder().encode('last in group')
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track3')
      const datagram = Datagram.newPayload(42n, 100n, 10n, 128, null, payload, true)

      expect(MoqtObject.isDatagramEndOfGroup(datagram)).toBe(true)

      const moqtObj = MoqtObject.fromDatagram(datagram, fullTrackName)
      const backToDatagram = moqtObj.tryIntoDatagram(42n, true)
      expect(backToDatagram.endOfGroup).toBe(true)
    })

    test('convert from/to Datagram status (Draft-16)', () => {
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track3')
      const datagram = Datagram.newStatus(42n, 100n, 10n, 128, null, ObjectStatus.EndOfGroup)

      const moqtObj = MoqtObject.fromDatagram(datagram, fullTrackName)
      expect(moqtObj.objectStatus).toBe(ObjectStatus.EndOfGroup)
      expect(moqtObj.payload).toBeNull()

      const backToDatagram = moqtObj.tryIntoDatagram(42n)
      expect(backToDatagram.objectStatus).toBe(ObjectStatus.EndOfGroup)
      expect(backToDatagram.payload).toBeNull()
    })

    test('convert Datagram with DEFAULT_PRIORITY (Draft-16)', () => {
      const payload = new TextEncoder().encode('default prio')
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track3')
      const datagram = Datagram.newPayload(42n, 100n, 10n, null, null, payload, false)

      const moqtObj = MoqtObject.fromDatagram(datagram, fullTrackName, 64)
      expect(moqtObj.publisherPriority).toBe(64) // resolved from default

      const backToDatagram = moqtObj.tryIntoDatagram(42n, false, 64)
      expect(backToDatagram.publisherPriority).toBeNull() // matches default, so null
    })

    test('convert from/to FetchObject', () => {
      const payload = new TextEncoder().encode('fetch payload')
      const fullTrackName = FullTrackName.tryNew('test/demo', 'track4')
      const fetchObj = FetchObject.newObject(
        100n,
        5n,
        10n,
        128,
        ObjectForwardingPreference.Subgroup,
        null,
        payload,
      )

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
