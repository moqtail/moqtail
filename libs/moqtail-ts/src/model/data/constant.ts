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
 * Object datagram status types for MOQT objects.
 * - `WithoutExtensions`: Object datagram without extensions.
 * - `WithExtensions`: Object datagram with extensions.
 */
export enum ObjectDatagramStatusType {
  WithoutExtensions = 0x02,
  WithExtensions = 0x03,
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
      case 0x02:
        return ObjectDatagramStatusType.WithoutExtensions
      case 0x03:
        return ObjectDatagramStatusType.WithExtensions
      default:
        throw new Error(`Invalid ObjectDatagramStatusType: ${value}`)
    }
  }
}

/**
 * @public
 * Object datagram types for MOQT objects.
 * - `WithoutExtensions`: Object datagram without extensions.
 * - `WithExtensions`: Object datagram with extensions.
 */
export enum ObjectDatagramType {
  WithoutExtensions = 0x00,
  WithExtensions = 0x01,
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
        return ObjectDatagramType.WithoutExtensions
      case 0x01:
        return ObjectDatagramType.WithExtensions
      default:
        throw new Error(`Invalid ObjectDatagramType: ${value}`)
    }
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
