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

/**
 * @public
 * Object datagram types for MOQT objects (Draft-16).
 *
 * Type bit layout (form 0b00X0XXXX):
 * - Bit 0 (0x01): EXTENSIONS - Extensions field present
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
  /** Mask for bits that must be zero: bits 4, 6, 7 (form `0b00X0XXXX`). */
  const INVALID_BITS_MASK = 0xd0

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
   * Returns true if the type has extensions (bit 0 set).
   * @param t - The ObjectDatagramType.
   */
  export function hasExtensions(t: ObjectDatagramType): boolean {
    return (t & 0x01) !== 0
  }

  /**
   * Returns true if the type indicates End of Group (bit 1 set).
   * @param t - The ObjectDatagramType.
   */
  export function isEndOfGroup(t: ObjectDatagramType): boolean {
    return (t & 0x02) !== 0
  }

  /**
   * Returns true if Object ID is absent (bit 2 set).
   * When true, Object ID is omitted and assumed to be 0.
   * @param t - The ObjectDatagramType.
   */
  export function isZeroObjectId(t: ObjectDatagramType): boolean {
    return (t & 0x04) !== 0
  }

  /**
   * Returns true if Publisher Priority is omitted (bit 3 set).
   * When true, the priority is inherited from the control message.
   * @param t - The ObjectDatagramType.
   */
  export function hasDefaultPriority(t: ObjectDatagramType): boolean {
    return (t & 0x08) !== 0
  }

  /**
   * Returns true if the datagram carries Object Status instead of payload (bit 5 set).
   * @param t - The ObjectDatagramType.
   */
  export function isStatus(t: ObjectDatagramType): boolean {
    return (t & 0x20) !== 0
  }

  /**
   * Determines the appropriate type for given properties.
   * @param hasExtensions - Whether extensions are present.
   * @param endOfGroup - Whether this is the last object in the group.
   * @param objectIdIsZero - Whether the objectId is 0.
   * @param defaultPriority - Whether publisher priority is inherited (omitted).
   * @param isStatus - Whether the datagram carries status instead of payload.
   * @throws Error if STATUS and END_OF_GROUP are both true (PROTOCOL_VIOLATION).
   */
  export function fromProperties(
    hasExtensions: boolean,
    endOfGroup: boolean,
    objectIdIsZero: boolean,
    defaultPriority: boolean,
    isStatus: boolean,
  ): ObjectDatagramType {
    if (isStatus && endOfGroup) {
      throw new Error('PROTOCOL_VIOLATION: STATUS and END_OF_GROUP cannot both be set')
    }
    let type = 0
    if (hasExtensions) type |= 0x01
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
 * Type bit layout (0b00X1XXXX):
 * - Bit 0 (0x01): EXTENSIONS - Extensions present in all objects
 * - Bits 1-2 (0x06): SUBGROUP_ID_MODE - How subgroup ID is encoded (0b00=zero, 0b01=firstObjId, 0b10=explicit, 0b11=invalid)
 * - Bit 3 (0x08): END_OF_GROUP - This subgroup contains the final object in the group
 * - Bit 4 (0x10): Always set (distinguishes subgroup from other header types)
 * - Bit 5 (0x20): DEFAULT_PRIORITY - Publisher priority field omitted, inherited from subscription
 *
 * Valid ranges: 0x10-0x15, 0x18-0x1D (bit 5=0), 0x30-0x35, 0x38-0x3D (bit 5=1)
 * Invalid: 0x16, 0x17, 0x1E, 0x1F, 0x36, 0x37, 0x3E, 0x3F (SUBGROUP_ID_MODE=0b11)
 */
export enum SubgroupHeaderType {
  // Bit 5 = 0 (priority present)
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
  // Bit 5 = 1 (default priority)
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
}

/**
 * Namespace for SubgroupHeaderType utilities and bit constants.
 */
export namespace SubgroupHeaderType {
  /** Extensions present in all objects (bit 0) */
  export const EXTENSIONS = 0x01
  /** Mask for SUBGROUP_ID_MODE (bits 1-2) */
  export const SUBGROUP_ID_MODE_MASK = 0x06
  /** This subgroup contains the final object in the group (bit 3) */
  export const END_OF_GROUP = 0x08
  /** Required bit that must always be set (bit 4) */
  export const REQUIRED_BIT = 0x10
  /** Publisher priority field omitted, inherited from subscription (bit 5) */
  export const DEFAULT_PRIORITY = 0x20
  /** Mask for bits that must be zero: bits 6-7 */
  const INVALID_BITS_MASK = 0xc0
  /** Reserved SUBGROUP_ID_MODE value (0b11) */
  const RESERVED_SUBGROUP_MODE = 0x06

  export function hasExtensions(t: SubgroupHeaderType): boolean {
    return (t & EXTENSIONS) !== 0
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

  /**
   * Converts a number or bigint to SubgroupHeaderType.
   * Validates bit 4 must be set, SUBGROUP_ID_MODE must not be 0b11.
   */
  export function tryFrom(value: number | bigint): SubgroupHeaderType {
    const v = typeof value === 'bigint' ? Number(value) : value

    if ((v & INVALID_BITS_MASK) !== 0) {
      throw new Error(`Invalid SubgroupHeaderType: 0x${v.toString(16)} (invalid bits set)`)
    }

    if ((v & REQUIRED_BIT) === 0) {
      throw new Error(`Invalid SubgroupHeaderType: 0x${v.toString(16)} (bit 4 not set)`)
    }

    if ((v & SUBGROUP_ID_MODE_MASK) === RESERVED_SUBGROUP_MODE) {
      throw new Error(`Invalid SubgroupHeaderType: 0x${v.toString(16)} (reserved SUBGROUP_ID_MODE)`)
    }

    return v as SubgroupHeaderType
  }

  /**
   * Determines the appropriate type for given properties.
   * @param subgroupIdMode - SUBGROUP_ID_MODE (0=zero, 1=firstObjId, 2=explicit).
   */
  export function fromProperties(
    hasExtensions: boolean,
    subgroupIdMode: 0 | 1 | 2,
    containsEndOfGroup: boolean,
    hasDefaultPriority: boolean = false,
  ): SubgroupHeaderType {
    let t = REQUIRED_BIT
    if (hasExtensions) t |= EXTENSIONS
    t |= (subgroupIdMode & 0x03) << 1
    if (containsEndOfGroup) t |= END_OF_GROUP
    if (hasDefaultPriority) t |= DEFAULT_PRIORITY
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
