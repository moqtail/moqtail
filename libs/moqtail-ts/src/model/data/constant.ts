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

import { InvalidTypeError, ProtocolViolationError } from '../error'

/**
 * @public
 * Object datagram types for MOQT objects (Draft-16).
 *
 * Type bit layout (form 0b00X0XXXX):
 * - Bit 0 (0x01): PROPERTIES - Properties field present
 * - Bit 1 (0x02): END_OF_GROUP - Last object in group
 * - Bit 2 (0x04): ZERO_OBJECT_ID - Object ID omitted (assumed 0)
 * - Bit 3 (0x08): DEFAULT_PRIORITY - Publisher Priority omitted (inherited)
 * - Bit 5 (0x20): STATUS - Object Status replaces Object Payload
 *
 * Invalid combinations:
 * - STATUS (0x20) + END_OF_GROUP (0x02) together is a PROTOCOL_VIOLATION
 * - Types outside the form 0b00X0XXXX are invalid
 */
export enum ObjectDatagramType {
  Type0x00 = 0x00,
  Type0x01 = 0x01,
  Type0x02 = 0x02,
  Type0x03 = 0x03,
  Type0x04 = 0x04,
  Type0x05 = 0x05,
  Type0x06 = 0x06,
  Type0x07 = 0x07,
  Type0x08 = 0x08,
  Type0x09 = 0x09,
  Type0x0A = 0x0a,
  Type0x0B = 0x0b,
  Type0x0C = 0x0c,
  Type0x0D = 0x0d,
  Type0x0E = 0x0e,
  Type0x0F = 0x0f,
  Type0x20 = 0x20,
  Type0x21 = 0x21,
  Type0x24 = 0x24,
  Type0x25 = 0x25,
  Type0x28 = 0x28,
  Type0x29 = 0x29,
  Type0x2C = 0x2c,
  Type0x2D = 0x2d,
}

/**
 * @public
 * Namespace for ObjectDatagramType utilities.
 */
export namespace ObjectDatagramType {
  /** Properties field present (bit 0). */
  export const PROPERTIES = 0x01
  /** Last object in the group (bit 1). */
  export const END_OF_GROUP = 0x02
  /** Object ID omitted, taken as 0 (bit 2). */
  export const ZERO_OBJECT_ID = 0x04
  /** Publisher Priority omitted, taken from the subscription (bit 3). */
  export const DEFAULT_PRIORITY = 0x08
  /** Carries an object status rather than a payload (bit 5). */
  export const STATUS = 0x20
  /** Mask for bits that must be zero: bits 4, 6, 7 (form `0b00X0XXXX`). */
  export const INVALID_BITS_MASK = 0xd0

  /**
   * Converts a number or bigint to ObjectDatagramType.
   * Validates using bitmask: must match form 0b00X0XXXX,
   * and STATUS + END_OF_GROUP cannot both be set.
   * @param value - The value to convert.
   * @returns The corresponding ObjectDatagramType.
   * @throws Error if the value is not valid.
   */
  export function tryFrom(value: number | bigint): ObjectDatagramType {
    const v = typeof value === 'bigint' ? Number(value) : value
    if ((v & INVALID_BITS_MASK) !== 0) {
      throw new Error(`Invalid ObjectDatagramType: ${value}, must match form 0b00X0XXXX`)
    }
    if ((v & 0x22) === 0x22) {
      throw new Error(`Invalid ObjectDatagramType: ${value}, STATUS and END_OF_GROUP cannot both be set`)
    }
    return v as ObjectDatagramType
  }

  /**
   * Returns true if the type has properties (bit 0 set).
   * @param t - The ObjectDatagramType.
   */
  export function hasProperties(t: ObjectDatagramType): boolean {
    return (t & PROPERTIES) !== 0
  }

  /**
   * Returns true if the type indicates End of Group (bit 1 set).
   * @param t - The ObjectDatagramType.
   */
  export function isEndOfGroup(t: ObjectDatagramType): boolean {
    return (t & END_OF_GROUP) !== 0
  }

  /**
   * Returns true if Object ID is absent (bit 2 set).
   * When true, Object ID is omitted and assumed to be 0.
   * @param t - The ObjectDatagramType.
   */
  export function isZeroObjectId(t: ObjectDatagramType): boolean {
    return (t & ZERO_OBJECT_ID) !== 0
  }

  /**
   * Returns true if Publisher Priority is omitted (bit 3 set).
   * When true, the priority is inherited from the control message.
   * @param t - The ObjectDatagramType.
   */
  export function hasDefaultPriority(t: ObjectDatagramType): boolean {
    return (t & DEFAULT_PRIORITY) !== 0
  }

  /**
   * Returns true if the datagram carries Object Status instead of payload (bit 5 set).
   * @param t - The ObjectDatagramType.
   */
  export function isStatus(t: ObjectDatagramType): boolean {
    return (t & STATUS) !== 0
  }

  /**
   * Determines the appropriate type for given properties.
   * @param hasProperties - Whether properties are present.
   * @param endOfGroup - Whether this is the last object in the group.
   * @param objectIdIsZero - Whether the objectId is 0.
   * @param defaultPriority - Whether publisher priority is inherited (omitted).
   * @param isStatus - Whether the datagram carries status instead of payload.
   * @throws Error if STATUS and END_OF_GROUP are both true (PROTOCOL_VIOLATION).
   */
  export function fromProperties(
    hasProperties: boolean,
    endOfGroup: boolean,
    objectIdIsZero: boolean,
    defaultPriority: boolean,
    isStatus: boolean,
  ): ObjectDatagramType {
    if (isStatus && endOfGroup) {
      throw new Error('PROTOCOL_VIOLATION: STATUS and END_OF_GROUP cannot both be set')
    }
    let type = 0
    if (hasProperties) type |= 0x01
    if (endOfGroup) type |= 0x02
    if (objectIdIsZero) type |= 0x04
    if (defaultPriority) type |= 0x08
    if (isStatus) type |= 0x20
    return type as ObjectDatagramType
  }
}

/**
 * @public
 * Fetch header types for MOQT fetch requests.
 */
export enum FetchHeaderType {
  Type0x05 = 0x05,
}

/**
 * Namespace for FetchHeaderType utilities.
 */
export namespace FetchHeaderType {
  /**
   * Converts a number or bigint to FetchHeaderType.
   * @param value - The value to convert.
   * @returns The corresponding FetchHeaderType.
   * @throws Error if the value is not valid.
   */
  export function tryFrom(value: number | bigint): FetchHeaderType {
    const v = typeof value === 'bigint' ? Number(value) : value
    switch (v) {
      case 0x05:
        return FetchHeaderType.Type0x05
      default:
        throw new Error(`Invalid FetchHeaderType: ${value}`)
    }
  }
}

/**
 * @public
 * Subgroup header types for MOQT subgroups.
 *
 * Type bit layout (0b0XX1XXXX):
 * - Bit 0 (0x01): PROPERTIES - Properties present in all objects
 * - Bits 1-2 (0x06): SUBGROUP_ID_MODE - How subgroup ID is encoded (0b00=zero, 0b01=firstObjId, 0b10=explicit, 0b11=invalid)
 * - Bit 3 (0x08): END_OF_GROUP - This subgroup contains the final object in the group
 * - Bit 4 (0x10): Always set (distinguishes subgroup from other header types)
 * - Bit 5 (0x20): DEFAULT_PRIORITY - Publisher priority field omitted, inherited from subscription
 * - Bit 6 (0x40): FIRST_OBJECT - The first object on this stream is the first the original
 *   publisher published in the subgroup
 * - Bit 7 (0x80): Must be zero
 *
 * Valid ranges: 0x10-0x15, 0x18-0x1D, 0x30-0x35, 0x38-0x3D, 0x50-0x55, 0x58-0x5D, 0x70-0x75, 0x78-0x7D
 * Invalid: 0x16, 0x17, 0x1E, 0x1F, 0x36, 0x37, 0x3E, 0x3F, 0x56, 0x57, 0x5E, 0x5F, 0x76, 0x77, 0x7E,
 * 0x7F (SUBGROUP_ID_MODE=0b11)
 */
export enum SubgroupHeaderType {
  // Bit 6 = 0 (not the subgroup's first object), bit 5 = 0 (priority present)
  Type0x10 = 0x10,
  Type0x11 = 0x11,
  Type0x12 = 0x12,
  Type0x13 = 0x13,
  Type0x14 = 0x14,
  Type0x15 = 0x15,
  Type0x18 = 0x18,
  Type0x19 = 0x19,
  Type0x1A = 0x1a,
  Type0x1B = 0x1b,
  Type0x1C = 0x1c,
  Type0x1D = 0x1d,
  // Bit 6 = 0, bit 5 = 1 (default priority)
  Type0x30 = 0x30,
  Type0x31 = 0x31,
  Type0x32 = 0x32,
  Type0x33 = 0x33,
  Type0x34 = 0x34,
  Type0x35 = 0x35,
  Type0x38 = 0x38,
  Type0x39 = 0x39,
  Type0x3A = 0x3a,
  Type0x3B = 0x3b,
  Type0x3C = 0x3c,
  Type0x3D = 0x3d,
  // Bit 6 = 1 (subgroup's first object), bit 5 = 0 (priority present)
  Type0x50 = 0x50,
  Type0x51 = 0x51,
  Type0x52 = 0x52,
  Type0x53 = 0x53,
  Type0x54 = 0x54,
  Type0x55 = 0x55,
  Type0x58 = 0x58,
  Type0x59 = 0x59,
  Type0x5A = 0x5a,
  Type0x5B = 0x5b,
  Type0x5C = 0x5c,
  Type0x5D = 0x5d,
  // Bit 6 = 1, bit 5 = 1 (default priority)
  Type0x70 = 0x70,
  Type0x71 = 0x71,
  Type0x72 = 0x72,
  Type0x73 = 0x73,
  Type0x74 = 0x74,
  Type0x75 = 0x75,
  Type0x78 = 0x78,
  Type0x79 = 0x79,
  Type0x7A = 0x7a,
  Type0x7B = 0x7b,
  Type0x7C = 0x7c,
  Type0x7D = 0x7d,
}

/**
 * Namespace for SubgroupHeaderType utilities and bit constants.
 */
/**
 * @public
 * The Publisher Priority to assume where a header omits the field, having set its
 * DEFAULT_PRIORITY bit. A track may name its own with the DEFAULT_PUBLISHER_PRIORITY
 * track property; this is the value for one that does not. Priority runs from 0 (most
 * important) to 255, so a wrong default here is not a small error: 0 makes every such
 * object outrank all others.
 */
export const DEFAULT_PUBLISHER_PRIORITY = 128

export namespace SubgroupHeaderType {
  /** Properties present in all objects (bit 0) */
  export const PROPERTIES = 0x01
  /** Mask for SUBGROUP_ID_MODE (bits 1-2) */
  export const SUBGROUP_ID_MODE_MASK = 0x06
  /** This subgroup contains the final object in the group (bit 3) */
  export const END_OF_GROUP = 0x08
  /** Required bit that must always be set (bit 4) */
  export const REQUIRED_BIT = 0x10
  /** Publisher priority field omitted, inherited from subscription (bit 5) */
  export const DEFAULT_PRIORITY = 0x20
  /** First object on this stream is the first published in the subgroup (bit 6) */
  export const FIRST_OBJECT = 0x40
  /** Mask for bits that must be zero: bit 7 only */
  export const INVALID_BITS_MASK = 0x80
  /** Reserved SUBGROUP_ID_MODE value (0b11) */
  const RESERVED_SUBGROUP_MODE = 0x06

  export function hasProperties(t: SubgroupHeaderType): boolean {
    return (t & PROPERTIES) !== 0
  }

  export function hasExplicitSubgroupId(t: SubgroupHeaderType): boolean {
    return (t & SUBGROUP_ID_MODE_MASK) === 0x04
  }

  export function isSubgroupIdZero(t: SubgroupHeaderType): boolean {
    return (t & SUBGROUP_ID_MODE_MASK) === 0x00
  }

  export function isSubgroupIdFirstObjectId(t: SubgroupHeaderType): boolean {
    return (t & SUBGROUP_ID_MODE_MASK) === 0x02
  }

  export function containsEndOfGroup(t: SubgroupHeaderType): boolean {
    return (t & END_OF_GROUP) !== 0
  }

  export function hasDefaultPriority(t: SubgroupHeaderType): boolean {
    return (t & DEFAULT_PRIORITY) !== 0
  }

  export function isFirstObject(t: SubgroupHeaderType): boolean {
    return (t & FIRST_OBJECT) !== 0
  }

  /**
   * Converts a number or bigint to SubgroupHeaderType.
   * Validates bit 7 must be zero, bit 4 must be set, SUBGROUP_ID_MODE must not be 0b11.
   */
  export function tryFrom(value: number | bigint): SubgroupHeaderType {
    const v = typeof value === 'bigint' ? Number(value) : value

    if (v < 0 || v > 0xff || (v & INVALID_BITS_MASK) !== 0) {
      throw new InvalidTypeError('SubgroupHeaderType.tryFrom', `invalid bits set, got 0x${v.toString(16)}`)
    }

    if ((v & REQUIRED_BIT) === 0) {
      throw new InvalidTypeError('SubgroupHeaderType.tryFrom', `bit 4 not set, got 0x${v.toString(16)}`)
    }

    if ((v & SUBGROUP_ID_MODE_MASK) === RESERVED_SUBGROUP_MODE) {
      throw new ProtocolViolationError(
        'SubgroupHeaderType.tryFrom',
        `reserved SUBGROUP_ID_MODE 0b11, got 0x${v.toString(16)}`,
      )
    }

    return v as SubgroupHeaderType
  }

  /**
   * Determines the appropriate type for given properties.
   * @param subgroupIdMode - SUBGROUP_ID_MODE (0=zero, 1=firstObjId, 2=explicit).
   * @param firstObject - Whether the first object on the stream is the first the original
   * publisher published in the subgroup.
   */
  export function fromProperties(
    hasProperties: boolean,
    subgroupIdMode: 0 | 1 | 2,
    containsEndOfGroup: boolean,
    hasDefaultPriority: boolean = false,
    firstObject: boolean = false,
  ): SubgroupHeaderType {
    let t = REQUIRED_BIT
    if (hasProperties) t |= PROPERTIES
    t |= (subgroupIdMode & 0x03) << 1
    if (containsEndOfGroup) t |= END_OF_GROUP
    if (hasDefaultPriority) t |= DEFAULT_PRIORITY
    if (firstObject) t |= FIRST_OBJECT
    return t as SubgroupHeaderType
  }
}

/**
 * @public
 * Publisher's preferred object delivery mechanism for a track.
 * - `Subgroup`: Use ordered subgroups (reliable).
 * - `Datagram`: Use unreliable datagrams when feasible.
 *
 * The preference is advisory: the relay/transport layer MAY override based on negotiated capabilities.
 */
export enum ObjectForwardingPreference {
  Subgroup = 'Subgroup',
  Datagram = 'Datagram',
}

/**
 * Namespace for ObjectForwardingPreference utilities.
 */
export namespace ObjectForwardingPreference {
  /**
   * Converts a number, bigint, or string to ObjectForwardingPreference.
   * @param value - The value to convert.
   * @returns The corresponding ObjectForwardingPreference.
   * @throws Error if the value is not valid.
   */
  export function tryFrom(value: number | bigint | string): ObjectForwardingPreference {
    if (value === 'Subgroup') return ObjectForwardingPreference.Subgroup
    if (value === 'Datagram') return ObjectForwardingPreference.Datagram
    throw new Error(`Invalid ObjectForwardingPreference: ${value}`)
  }
}

/**
 * @public
 * Object status codes for MOQT objects.
 * - `Normal`: Object exists and is available.
 * - `EndOfGroup`: End of group marker.
 * - `EndOfTrack`: End of track marker.
 */
export enum ObjectStatus {
  Normal = 0x0,
  EndOfGroup = 0x3,
  EndOfTrack = 0x4,
}

/**
 * Namespace for ObjectStatus utilities.
 */
export namespace ObjectStatus {
  /**
   * Converts a number or bigint to ObjectStatus.
   * @param value - The value to convert.
   * @returns The corresponding ObjectStatus.
   * @throws Error if the value is not valid.
   */
  export function tryFrom(value: number | bigint): ObjectStatus {
    const v = typeof value === 'bigint' ? Number(value) : value
    switch (v) {
      case 0x0:
        return ObjectStatus.Normal
      case 0x3:
        return ObjectStatus.EndOfGroup
      case 0x4:
        return ObjectStatus.EndOfTrack
      default:
        throw new Error(`Invalid ObjectStatus: ${value}`)
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  // The type bytes are checked against dev/conformance/draft18/data_stream_types.json,
  // which is shared with moqtail-rs. The bit layout and the set of accepted bytes live
  // there, not in this file.
  describe('data stream type conformance', () => {
    const fixture = async () => await import('../../../test/conformance')

    test('SubgroupHeaderType bits match the fixture', async () => {
      const { dataStreamTypes, bitMask } = await fixture()
      const t = dataStreamTypes().subgroup_header
      expect(bitMask(t, 'PROPERTIES')).toBe(SubgroupHeaderType.PROPERTIES)
      expect(bitMask(t, 'SUBGROUP_ID_MODE')).toBe(SubgroupHeaderType.SUBGROUP_ID_MODE_MASK)
      expect(bitMask(t, 'END_OF_GROUP')).toBe(SubgroupHeaderType.END_OF_GROUP)
      expect(bitMask(t, 'REQUIRED')).toBe(SubgroupHeaderType.REQUIRED_BIT)
      expect(bitMask(t, 'DEFAULT_PRIORITY')).toBe(SubgroupHeaderType.DEFAULT_PRIORITY)
      expect(bitMask(t, 'FIRST_OBJECT')).toBe(SubgroupHeaderType.FIRST_OBJECT)
    })

    test('SubgroupHeaderType accepts exactly the fixture bytes', async () => {
      const { dataStreamTypes, validBytes } = await fixture()
      const valid = validBytes(dataStreamTypes().subgroup_header)
      for (let b = 0; b <= 0xff; b++) {
        let accepted = true
        try {
          SubgroupHeaderType.tryFrom(b)
        } catch {
          accepted = false
        }
        expect([b, accepted]).toEqual([b, valid.includes(b)])
      }
    })

    test('subgroup id modes match the fixture', async () => {
      const { dataStreamTypes, parseHex } = await fixture()
      const t = dataStreamTypes().subgroup_header
      for (const mode of t.subgroup_id_modes ?? []) {
        const bits = Number(parseHex(mode.value))
        const byte = SubgroupHeaderType.REQUIRED_BIT | bits
        if (mode.reserved) {
          expect(() => SubgroupHeaderType.tryFrom(byte)).toThrow()
          continue
        }
        const parsed = SubgroupHeaderType.tryFrom(byte)
        if (mode.name === 'ZERO') expect(SubgroupHeaderType.isSubgroupIdZero(parsed)).toBe(true)
        else if (mode.name === 'FIRST_OBJECT_ID')
          expect(SubgroupHeaderType.isSubgroupIdFirstObjectId(parsed)).toBe(true)
        else if (mode.name === 'EXPLICIT') expect(SubgroupHeaderType.hasExplicitSubgroupId(parsed)).toBe(true)
        else throw new Error(`fixture names an unknown subgroup id mode ${mode.name}`)
      }
    })

    test('ObjectDatagramType bits match the fixture', async () => {
      const { dataStreamTypes, bitMask } = await fixture()
      const t = dataStreamTypes().object_datagram
      expect(bitMask(t, 'PROPERTIES')).toBe(ObjectDatagramType.PROPERTIES)
      expect(bitMask(t, 'END_OF_GROUP')).toBe(ObjectDatagramType.END_OF_GROUP)
      expect(bitMask(t, 'ZERO_OBJECT_ID')).toBe(ObjectDatagramType.ZERO_OBJECT_ID)
      expect(bitMask(t, 'DEFAULT_PRIORITY')).toBe(ObjectDatagramType.DEFAULT_PRIORITY)
      expect(bitMask(t, 'STATUS')).toBe(ObjectDatagramType.STATUS)
    })

    test('ObjectDatagramType accepts exactly the fixture bytes', async () => {
      const { dataStreamTypes, validBytes } = await fixture()
      const valid = validBytes(dataStreamTypes().object_datagram)
      for (let b = 0; b <= 0xff; b++) {
        let accepted = true
        try {
          ObjectDatagramType.tryFrom(b)
        } catch {
          accepted = false
        }
        expect([b, accepted]).toEqual([b, valid.includes(b)])
      }
    })
  })
  describe('SubgroupHeaderType', () => {
    // Draft-18 form 0b0XX1XXXX, minus the reserved SUBGROUP_ID_MODE 0b11.
    const isValid = (b: number) => (b & 0x80) === 0 && (b & 0x10) !== 0 && (b & 0x06) !== 0x06

    test('classifies all 256 type bytes', () => {
      const accepted: number[] = []
      for (let b = 0; b <= 0xff; b++) {
        let ok = true
        try {
          expect(SubgroupHeaderType.tryFrom(b)).toBe(b)
        } catch {
          ok = false
        }
        if (ok) accepted.push(b)
        expect([b, ok]).toEqual([b, isValid(b)])
      }
      expect(accepted.length).toBe(48)
      const declared = Object.entries(SubgroupHeaderType)
        .filter(([k, v]) => k.startsWith('Type0x') && typeof v === 'number')
        .map(([, v]) => v as number)
      expect(accepted).toEqual(declared)
    })

    test('rejects reserved SUBGROUP_ID_MODE with a protocol violation', () => {
      for (const b of [0x16, 0x17, 0x1e, 0x1f, 0x36, 0x37, 0x3e, 0x3f, 0x56, 0x57, 0x5e, 0x5f, 0x76, 0x77, 0x7e, 0x7f])
        expect(() => SubgroupHeaderType.tryFrom(b)).toThrow(ProtocolViolationError)
    })

    test('bit accessors agree with the type byte', () => {
      for (let b = 0; b <= 0xff; b++) {
        if (!isValid(b)) continue
        const t = SubgroupHeaderType.tryFrom(b)
        expect(SubgroupHeaderType.hasProperties(t)).toBe((b & 0x01) !== 0)
        expect(SubgroupHeaderType.isSubgroupIdZero(t)).toBe((b & 0x06) === 0x00)
        expect(SubgroupHeaderType.isSubgroupIdFirstObjectId(t)).toBe((b & 0x06) === 0x02)
        expect(SubgroupHeaderType.hasExplicitSubgroupId(t)).toBe((b & 0x06) === 0x04)
        expect(SubgroupHeaderType.containsEndOfGroup(t)).toBe((b & 0x08) !== 0)
        expect(SubgroupHeaderType.hasDefaultPriority(t)).toBe((b & 0x20) !== 0)
        expect(SubgroupHeaderType.isFirstObject(t)).toBe((b & 0x40) !== 0)
      }
    })

    test('fromProperties round-trips every valid type byte', () => {
      for (let b = 0; b <= 0xff; b++) {
        if (!isValid(b)) continue
        const t = SubgroupHeaderType.fromProperties(
          (b & 0x01) !== 0,
          ((b & 0x06) >> 1) as 0 | 1 | 2,
          (b & 0x08) !== 0,
          (b & 0x20) !== 0,
          (b & 0x40) !== 0,
        )
        expect(t).toBe(b)
      }
    })

    test('FIRST_OBJECT defaults to unset', () => {
      expect(SubgroupHeaderType.fromProperties(false, 2, false)).toBe(0x14)
      expect(SubgroupHeaderType.fromProperties(false, 2, false, false, true)).toBe(0x54)
    })
  })
}
