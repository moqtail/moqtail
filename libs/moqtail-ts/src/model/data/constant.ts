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
 * Object datagram status types for MOQT objects (Draft-14).
 * Status datagrams use types 0x20-0x21.
 *
 * Type bit layout:
 * - Bit 0: Extensions Present (0 = no, 1 = yes)
 *
 * | Type | Extensions Present | Object ID Present |
 * |------|-------------------|------------------|
 * | 0x20 | No                | Yes              |
 * | 0x21 | Yes               | Yes              |
 */
export enum ObjectDatagramStatusType {
  /** Status without extensions (0x20) */
  WithoutExtensions = 0x20,
  /** Status with extensions (0x21) */
  WithExtensions = 0x21,
}

/**
 * @public
 * Namespace for ObjectDatagramStatusType utilities.
 */
export namespace ObjectDatagramStatusType {
  /**
   * Converts a number or bigint to ObjectDatagramStatusType.
   * @param value - The value to convert.
   * @returns The corresponding ObjectDatagramStatusType.
   * @throws Error if the value is not valid.
   */
  export function tryFrom(value: number | bigint): ObjectDatagramStatusType {
    const v = typeof value === 'bigint' ? Number(value) : value
    switch (v) {
      case 0x20:
        return ObjectDatagramStatusType.WithoutExtensions
      case 0x21:
        return ObjectDatagramStatusType.WithExtensions
      default:
        throw new Error(`Invalid ObjectDatagramStatusType: ${value}`)
    }
  }

  /**
   * Returns true if the type has extensions.
   * @param t - The ObjectDatagramStatusType.
   */
  export function hasExtensions(t: ObjectDatagramStatusType): boolean {
    return t === ObjectDatagramStatusType.WithExtensions
  }
}

/**
 * @public
 * Object datagram types for MOQT objects (Draft-14).
 *
 * Type bit layout for 0x00-0x07:
 * - Bit 0: Extensions Present (0 = no, 1 = yes)
 * - Bit 1: End of Group (0 = no, 1 = yes)
 * - Bit 2: Object ID Present (0 = Object ID omitted & is 0, 1 = Object ID present)
 *
 * Note: Bit 2 is inverted - when set, Object ID is ABSENT (and assumed 0)
 *
 * | Type | End of Group | Extensions | Object ID Present | Content |
 * |------|-------------|------------|------------------|--------|
 * | 0x00 | No          | No         | Yes              | Payload |
 * | 0x01 | No          | Yes        | Yes              | Payload |
 * | 0x02 | Yes         | No         | Yes              | Payload |
 * | 0x03 | Yes         | Yes        | Yes              | Payload |
 * | 0x04 | No          | No         | No (ID=0)        | Payload |
 * | 0x05 | No          | Yes        | No (ID=0)        | Payload |
 * | 0x06 | Yes         | No         | No (ID=0)        | Payload |
 * | 0x07 | Yes         | Yes        | No (ID=0)        | Payload |
 */
export enum ObjectDatagramType {
  /** No End of Group, No Extensions, Object ID Present (0x00) */
  Type0x00 = 0x00,
  /** No End of Group, With Extensions, Object ID Present (0x01) */
  Type0x01 = 0x01,
  /** End of Group, No Extensions, Object ID Present (0x02) */
  Type0x02 = 0x02,
  /** End of Group, With Extensions, Object ID Present (0x03) */
  Type0x03 = 0x03,
  /** No End of Group, No Extensions, Object ID = 0 (0x04) */
  Type0x04 = 0x04,
  /** No End of Group, With Extensions, Object ID = 0 (0x05) */
  Type0x05 = 0x05,
  /** End of Group, No Extensions, Object ID = 0 (0x06) */
  Type0x06 = 0x06,
  /** End of Group, With Extensions, Object ID = 0 (0x07) */
  Type0x07 = 0x07,
}

/**
 * @public
 * Namespace for ObjectDatagramType utilities.
 */
export namespace ObjectDatagramType {
  /**
   * Converts a number or bigint to ObjectDatagramType.
   * @param value - The value to convert.
   * @returns The corresponding ObjectDatagramType.
   * @throws Error if the value is not valid.
   */
  export function tryFrom(value: number | bigint): ObjectDatagramType {
    const v = typeof value === 'bigint' ? Number(value) : value
    switch (v) {
      case 0x00:
        return ObjectDatagramType.Type0x00
      case 0x01:
        return ObjectDatagramType.Type0x01
      case 0x02:
        return ObjectDatagramType.Type0x02
      case 0x03:
        return ObjectDatagramType.Type0x03
      case 0x04:
        return ObjectDatagramType.Type0x04
      case 0x05:
        return ObjectDatagramType.Type0x05
      case 0x06:
        return ObjectDatagramType.Type0x06
      case 0x07:
        return ObjectDatagramType.Type0x07
      default:
        throw new Error(`Invalid ObjectDatagramType: ${value}`)
    }
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
   * Returns true if Object ID is present in the wire format.
   * When bit 2 is set (0x04-0x07), Object ID is ABSENT and assumed to be 0.
   * @param t - The ObjectDatagramType.
   */
  export function hasObjectId(t: ObjectDatagramType): boolean {
    return (t & 0x04) === 0
  }

  /**
   * Determines the appropriate type for given properties.
   * @param hasExtensions - Whether extensions are present.
   * @param endOfGroup - Whether this is the last object in the group.
   * @param objectIdIsZero - Whether the objectId is 0.
   */
  export function fromProperties(
    hasExtensions: boolean,
    endOfGroup: boolean,
    objectIdIsZero: boolean,
  ): ObjectDatagramType {
    let type = 0
    if (hasExtensions) type |= 0x01
    if (endOfGroup) type |= 0x02
    if (objectIdIsZero) type |= 0x04
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
 * Subgroup header types for MOQT subgroups (Draft-16).
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
 * Namespace for SubgroupHeaderType utilities.
 */
export namespace SubgroupHeaderType {
  /**
   * Returns true if the header type has extensions (bit 0 set).
   * @param t - The SubgroupHeaderType.
   */
  export function hasExtensions(t: SubgroupHeaderType): boolean {
    return (t & 0x01) !== 0
  }

  /**
   * Returns true if the header type has an explicit subgroup ID (bits 1-2 = 0b10).
   * @param t - The SubgroupHeaderType.
   */
  export function hasExplicitSubgroupId(t: SubgroupHeaderType): boolean {
    return (t & 0x06) === 0x04
  }

  /**
   * Returns true if the header type implies a subgroup ID of zero (bits 1-2 = 0b00).
   * @param t - The SubgroupHeaderType.
   */
  export function isSubgroupIdZero(t: SubgroupHeaderType): boolean {
    return (t & 0x06) === 0x00
  }

  /**
   * Returns true if the header type implies subgroup ID is the first Object ID (bits 1-2 = 0b01).
   * @param t - The SubgroupHeaderType.
   */
  export function isSubgroupIdFirstObjectId(t: SubgroupHeaderType): boolean {
    return (t & 0x06) === 0x02
  }

  /**
   * Returns true if the header type indicates End of Group (bit 3 set).
   * @param t - The SubgroupHeaderType.
   */
  export function containsEndOfGroup(t: SubgroupHeaderType): boolean {
    return (t & 0x08) !== 0
  }

  /**
   * Returns true if the header type uses default publisher priority (bit 5 set).
   * When true, the publisher_priority field is omitted from the header.
   * @param t - The SubgroupHeaderType.
   */
  export function hasDefaultPriority(t: SubgroupHeaderType): boolean {
    return (t & 0x20) !== 0
  }

  /**
   * Converts a number or bigint to SubgroupHeaderType.
   * Validates per MOQ draft-16: bit 4 must be set, SUBGROUP_ID_MODE must not be 0b11.
   * @param value - The value to convert.
   * @returns The corresponding SubgroupHeaderType.
   * @throws Error if the value is not valid.
   */
  export function tryFrom(value: number | bigint): SubgroupHeaderType {
    const v = typeof value === 'bigint' ? Number(value) : value

    // Bit 4 (0x10) must be set
    if ((v & 0x10) === 0) {
      throw new Error(`Invalid SubgroupHeaderType: 0x${v.toString(16)} (bit 4 not set)`)
    }

    // SUBGROUP_ID_MODE (bits 1-2) must not be 0b11 (0x06)
    if ((v & 0x06) === 0x06) {
      throw new Error(`Invalid SubgroupHeaderType: 0x${v.toString(16)} (reserved SUBGROUP_ID_MODE)`)
    }

    // Must be in valid range
    if (v < 0 || v > 0x3f) {
      throw new Error(`Invalid SubgroupHeaderType: 0x${v.toString(16)} (out of range)`)
    }

    return v as SubgroupHeaderType
  }

  /**
   * Determines the appropriate type for given properties.
   * Bits 1-2 encode SUBGROUP_ID_MODE: 0b00=zero, 0b01=firstObjId, 0b10=explicit.
   * @param hasExtensions - Whether extensions are present (bit 0).
   * @param subgroupIdMode - SUBGROUP_ID_MODE (0, 1, or 2; represents 0b00, 0b01, 0b10).
   * @param containsEndOfGroup - Whether this is end of group (bit 3).
   * @param hasDefaultPriority - Whether to use default publisher priority (bit 5).
   */
  export function fromProperties(
    hasExtensions: boolean,
    subgroupIdMode: 0 | 1 | 2,
    containsEndOfGroup: boolean,
    hasDefaultPriority: boolean = false,
  ): SubgroupHeaderType {
    let type = 0x10 // bit 4 always set
    if (hasExtensions) type |= 0x01
    type |= (subgroupIdMode & 0x03) << 1 // bits 1-2
    if (containsEndOfGroup) type |= 0x08
    if (hasDefaultPriority) type |= 0x20
    return type as SubgroupHeaderType
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
 * - `DoesNotExist`: Object does not exist.
 * - `EndOfGroup`: End of group marker.
 * - `EndOfTrack`: End of track marker.
 */
export enum ObjectStatus {
  Normal = 0x0,
  DoesNotExist = 0x1,
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
      case 0x1:
        return ObjectStatus.DoesNotExist
      case 0x3:
        return ObjectStatus.EndOfGroup
      case 0x4:
        return ObjectStatus.EndOfTrack
      default:
        throw new Error(`Invalid ObjectStatus: ${value}`)
    }
  }
}

/**
 * @public
 * Fetch Object Serialization Flags (Draft-16 §10.4.4.1)
 *
 * Bitmask field controlling the wire format of Fetch Objects.
 * Valid values: 0x00-0x7F, 0x8C (End of Non-Existent Range), 0x10C (End of Unknown Range).
 *
 * Bit layout (values < 128):
 * - bits 1-0 (0x03): SubgroupIdMode (0=zero, 1=prior, 2=prior+1, 3=explicit)
 * - bit 2   (0x04): Object ID explicit (0 = prior+1)
 * - bit 3   (0x08): Group ID explicit (0 = same as prior)
 * - bit 4   (0x10): Publisher Priority explicit (0 = same as prior)
 * - bit 5   (0x20): Extensions present
 * - bit 6   (0x40): Datagram mode (bits 0-1 ignored, no Subgroup ID)
 */
export namespace FetchObjectSerializationFlags {
  export const END_OF_NON_EXISTENT_RANGE = 0x8c as const
  export const END_OF_UNKNOWN_RANGE = 0x10c as const

  /**
   * Validates and converts a value to FetchObjectSerializationFlags.
   * Throws ProtocolViolationError for invalid values.
   */
  export function tryFrom(value: number | bigint): number {
    const v = typeof value === 'bigint' ? Number(value) : value
    // Valid: 0x00-0x7F, 0x8C, 0x10C
    if ((v >= 0 && v <= 0x7f) || v === 0x8c || v === 0x10c) {
      return v
    }
    throw new Error(`Invalid FetchObjectSerializationFlags: 0x${v.toString(16)}`)
  }

  /**
   * Returns true if the flags represent an end-of-range marker.
   */
  export function isEndOfRange(flags: number): boolean {
    return flags === END_OF_NON_EXISTENT_RANGE || flags === END_OF_UNKNOWN_RANGE
  }

  /**
   * Extracts the SubgroupIdMode from flags (bits 0-1).
   * @returns 0=zero, 1=prior, 2=prior+1, 3=explicit
   */
  export function subgroupMode(flags: number): number {
    return flags & 0x03
  }

  /**
   * Returns true if Object ID field is explicit (bit 2 set).
   */
  export function hasExplicitObjectId(flags: number): boolean {
    return (flags & 0x04) !== 0
  }

  /**
   * Returns true if Group ID field is explicit (bit 3 set).
   */
  export function hasExplicitGroupId(flags: number): boolean {
    return (flags & 0x08) !== 0
  }

  /**
   * Returns true if Publisher Priority field is explicit (bit 4 set).
   */
  export function hasExplicitPriority(flags: number): boolean {
    return (flags & 0x10) !== 0
  }

  /**
   * Returns true if Extensions field is present (bit 5 set).
   */
  export function hasExtensions(flags: number): boolean {
    return (flags & 0x20) !== 0
  }

  /**
   * Returns true if this is a datagram-forwarded object (bit 6 set).
   * When set, bits 0-1 are ignored and no Subgroup ID is present.
   */
  export function isDatagram(flags: number): boolean {
    return (flags & 0x40) !== 0
  }

  /**
   * Computes optimal serialization flags for a FetchObject given prior state.
   * Performs delta encoding: omits fields when they can be inferred from prior.
   */
  export function fromProperties(
    prior: { groupId: bigint; subgroupId: bigint | null; objectId: bigint; publisherPriority: number } | undefined,
    obj: {
      subgroupId: bigint | null
      objectId: bigint
      publisherPriority: number
      extensionHeaders: unknown[] | null
      groupId: bigint
    },
  ): number {
    let flags = 0

    // Group ID: explicit if no prior or different
    if (!prior || prior.groupId !== obj.groupId) {
      flags |= 0x08
    }

    // Object ID: explicit if no prior or not sequential
    if (!prior || prior.objectId + 1n !== obj.objectId) {
      flags |= 0x04
    }

    // Publisher Priority: explicit if no prior or different
    if (!prior || prior.publisherPriority !== obj.publisherPriority) {
      flags |= 0x10
    }

    // Extensions: present if non-empty
    if (obj.extensionHeaders && obj.extensionHeaders.length > 0) {
      flags |= 0x20
    }

    // Subgroup ID mode: choose most compact representation
    if (obj.subgroupId === null) {
      // Datagram-forwarded object
      flags |= 0x40
      // bits 0-1 should be 0 for datagram
    } else {
      const mode = (() => {
        if (obj.subgroupId === 0n) {
          return 0x00 // zero
        } else if (prior && prior.subgroupId === obj.subgroupId) {
          return 0x01 // same as prior
        } else if (prior && prior.subgroupId !== null && prior.subgroupId + 1n === obj.subgroupId) {
          return 0x02 // prior + 1
        } else {
          return 0x03 // explicit
        }
      })()
      flags |= mode
    }

    return flags
  }
}
