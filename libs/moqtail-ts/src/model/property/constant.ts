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

import { InvalidTypeError } from '../error'

export enum LOCPropertyId {
  Timestamp = 0x06,
  Timescale = 0x08,
  VideoFrameMarking = 0x0a,
  AudioLevel = 0x0c,
  VideoConfig = 0x0d,
}

export enum TrackPropertyType {
  ObjectDeliveryTimeout = 0x02,
  MaxCacheDuration = 0x04,
  SubgroupDeliveryTimeout = 0x06,
  ImmutableProperties = 0x0b,
  DefaultPublisherPriority = 0x0e,
  DefaultPublisherGroupOrder = 0x22,
  DynamicGroups = 0x30,
  PriorGroupIdGap = 0x3c,
  PriorObjectIdGap = 0x3e,
}

export function locPropertyIdFromNumber(value: number): LOCPropertyId {
  switch (value) {
    case 0x06:
      return LOCPropertyId.Timestamp
    case 0x08:
      return LOCPropertyId.Timescale
    case 0x0a:
      return LOCPropertyId.VideoFrameMarking
    case 0x0c:
      return LOCPropertyId.AudioLevel
    case 0x0d:
      return LOCPropertyId.VideoConfig
    default:
      throw new InvalidTypeError('locPropertyIdFromNumber', `Invalid LOC property id: ${value}`)
  }
}

/**
 * @public
 * A registration-policy range in the Property Type space. `to` is absent for the
 * open-ended top range.
 */
export interface PropertyRange {
  readonly from: bigint
  readonly to?: bigint
}

/**
 * @public
 * Registration-policy ranges for the Property Type space.
 *
 */
export const PropertyRanges = {
  /** Standards Action or IESG Approval (1-byte encoding). */
  StandardsAction: { from: 0x00n, to: 0x77n },
  /** Application-specific use, no registration permitted (1-byte encoding). */
  AppSpecific1Byte: { from: 0x78n, to: 0x7fn },
  /** Specification Required (2-byte encoding). */
  SpecRequired: { from: 0x80n, to: 0x37ffn },
  /** Application-specific use, no registration permitted (2-byte encoding). */
  AppSpecific2Byte: { from: 0x3800n, to: 0x3fffn },
  /** Mandatory Track Properties; Track scope only. */
  MandatoryTrack: { from: 0x4000n, to: 0x7fffn },
  /** First Come First Served begins here (open-ended). */
  Fcfs: { from: 0x8000n },
} as const satisfies Record<string, PropertyRange>

function inRange(range: PropertyRange, typeValue: bigint): boolean {
  return typeValue >= range.from && (range.to === undefined || typeValue <= range.to)
}

/**
 * @public
 * True if a Property Type is reserved for application-specific use (either encoding-width
 * range), for which no IANA registration is permitted.
 */
export function isApplicationSpecificProperty(typeValue: bigint | number): boolean {
  const v = BigInt(typeValue)
  return inRange(PropertyRanges.AppSpecific1Byte, v) || inRange(PropertyRanges.AppSpecific2Byte, v)
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('application-specific property ranges', () => {
    test('covers both encoding widths and nothing either side of them', () => {
      for (const v of [0x78n, 0x7fn, 0x3800n, 0x3fffn]) {
        expect(isApplicationSpecificProperty(v), `${v.toString(16)}`).toBe(true)
      }
    })
  })
}
