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
 * Subgroup header types for MOQT subgroups.
 */
export enum SubgroupHeaderType {
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
}

/**
 * Namespace for SubgroupHeaderType utilities.
 */
export namespace SubgroupHeaderType {
  /**
   * Returns true if the header type has an explicit subgroup ID.
   * @param t - The SubgroupHeaderType.
   */
  export function hasExplicitSubgroupId(t: SubgroupHeaderType): boolean {
    switch (t) {
      case SubgroupHeaderType.Type0x14:
      case SubgroupHeaderType.Type0x15:
      case SubgroupHeaderType.Type0x1C:
      case SubgroupHeaderType.Type0x1D:
        return true
      default:
        return false
    }
  }
  /**
   * Returns true if the header type implies a subgroup ID of zero.
   * @param t - The SubgroupHeaderType.
   */
  export function isSubgroupIdZero(t: SubgroupHeaderType): boolean {
    switch (t) {
      case SubgroupHeaderType.Type0x10:
      case SubgroupHeaderType.Type0x11:
      case SubgroupHeaderType.Type0x18:
      case SubgroupHeaderType.Type0x19:
        return true
      default:
        return false
    }
  }
  /**
   * Returns true if the header type has extensions.
   * @param t - The SubgroupHeaderType.
   */
  export function hasExtensions(t: SubgroupHeaderType): boolean {
    switch (t) {
      case SubgroupHeaderType.Type0x11:
      case SubgroupHeaderType.Type0x13:
      case SubgroupHeaderType.Type0x15:
      case SubgroupHeaderType.Type0x19:
      case SubgroupHeaderType.Type0x1B:
      case SubgroupHeaderType.Type0x1D:
        return true
      default:
        return false
    }
  }
  /**
   * Converts a number or bigint to SubgroupHeaderType.
   * @param value - The value to convert.
   * @returns The corresponding SubgroupHeaderType.
   * @throws Error if the value is not valid.
   */
  export function tryFrom(value: number | bigint): SubgroupHeaderType {
    const v = typeof value === 'bigint' ? Number(value) : value
    switch (v) {
      case 0x10:
        return SubgroupHeaderType.Type0x10
      case 0x11:
        return SubgroupHeaderType.Type0x11
      case 0x12:
        return SubgroupHeaderType.Type0x12
      case 0x13:
        return SubgroupHeaderType.Type0x13
      case 0x14:
        return SubgroupHeaderType.Type0x14
      case 0x15:
        return SubgroupHeaderType.Type0x15
      case 0x18:
        return SubgroupHeaderType.Type0x18
      case 0x19:
        return SubgroupHeaderType.Type0x19
      case 0x1a:
        return SubgroupHeaderType.Type0x1A
      case 0x1b:
        return SubgroupHeaderType.Type0x1B
      case 0x1c:
        return SubgroupHeaderType.Type0x1C
      case 0x1d:
        return SubgroupHeaderType.Type0x1D
      default:
        throw new Error(`Invalid SubgroupHeaderType: ${value}`)
    }
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
