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
 * Registration-policy ranges for the Property Type space (§15.8).
 *
 * §2.5 gives the 1-byte application-specific range as `0x38-0x3F`; that is stale text
 * from before draft-18's varint change, which made the 1-byte space `0x00-0x7F`.
 * §15.8 is the correct table and is what these constants follow.
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

  // Asserted against dev/conformance/draft18/, which is shared with moqtail-rs.
  describe('draft-18 conformance', () => {
    const fixture = async () => await import('../../../test/conformance')

    test('TrackPropertyType matches property_types.json', async () => {
      const { propertyTypes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(propertyTypes(), pascalIdent(), (codepoint) => TrackPropertyType[Number(codepoint)])
    })

    // The LOC properties are registered in the same number space as the draft's own
    // table (§15.8 Table 15), so they are held to it too.
    test('LOCPropertyId matches the provisional LOC registry', async () => {
      const { propertyTypes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(propertyTypes().provisional, pascalIdent(), (codepoint) => {
        try {
          return LOCPropertyId[locPropertyIdFromNumber(Number(codepoint))]
        } catch {
          return undefined
        }
      })
    })

    // The ranges, in order, including the open-ended First Come First Served one.
    test('PropertyRanges matches the ranges in property_types.json', async () => {
      const { propertyTypes, parseHex } = await fixture()
      const actual = Object.values(PropertyRanges).map((r) => [r.from, 'to' in r ? r.to : undefined])
      const expected = propertyTypes().ranges.entries.map((e) => [
        parseHex(e.from),
        e.to === null ? undefined : parseHex(e.to),
      ])
      expect(actual).toEqual(expected)
    })
  })

  describe('application-specific property ranges', () => {
    test('covers both encoding widths and nothing either side of them', () => {
      for (const v of [0x78n, 0x7fn, 0x3800n, 0x3fffn]) {
        expect(isApplicationSpecificProperty(v), `${v.toString(16)}`).toBe(true)
      }
      // 0x38-0x3F is §2.5's stale range and must not be treated as application-specific.
      for (const v of [0x38n, 0x3fn, 0x77n, 0x80n, 0x37ffn, 0x4000n]) {
        expect(isApplicationSpecificProperty(v), `${v.toString(16)}`).toBe(false)
      }
    })
  })
}
