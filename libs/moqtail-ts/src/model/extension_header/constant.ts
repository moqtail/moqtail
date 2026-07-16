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

export enum LOCHeaderExtensionId {
  Timestamp = 0x06,
  Timescale = 0x08,
  VideoFrameMarking = 0x0a,
  AudioLevel = 0x0c,
  VideoConfig = 0x0d,
}

export enum TrackExtensionType {
  DeliveryTimeout = 0x02,
  MaxCacheDuration = 0x04,
  ImmutableExtensions = 0x0b,
  DefaultPublisherPriority = 0x0e,
  DefaultPublisherGroupOrder = 0x22,
  DynamicGroups = 0x30,
  PriorGroupIdGap = 0x3c,
  PriorObjectIdGap = 0x3e,
}

export function locHeaderExtensionIdFromNumber(value: number): LOCHeaderExtensionId {
  switch (value) {
    case 0x06:
      return LOCHeaderExtensionId.Timestamp
    case 0x08:
      return LOCHeaderExtensionId.Timescale
    case 0x0a:
      return LOCHeaderExtensionId.VideoFrameMarking
    case 0x0c:
      return LOCHeaderExtensionId.AudioLevel
    case 0x0d:
      return LOCHeaderExtensionId.VideoConfig
    default:
      throw new InvalidTypeError('locHeaderExtensionIdFromNumber', `Invalid LOC header extension id: ${value}`)
  }
}

if (import.meta.vitest) {
  const { describe, test } = import.meta.vitest

  // Asserted against dev/conformance/draft18/, which is shared with moqtail-rs.
  describe('draft-18 conformance', () => {
    const fixture = async () => await import('../../../test/conformance')

    test('TrackExtensionType matches property_types.json', async () => {
      const { propertyTypes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(propertyTypes(), pascalIdent(), (codepoint) => TrackExtensionType[Number(codepoint)])
    })

    // The LOC properties are registered in the same number space as the draft's own
    // table (§15.8 Table 15), so they are held to it too.
    test('LOCHeaderExtensionId matches the provisional LOC registry', async () => {
      const { propertyTypes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(propertyTypes().provisional, pascalIdent(), (codepoint) => {
        try {
          return LOCHeaderExtensionId[locHeaderExtensionIdFromNumber(Number(codepoint))]
        } catch {
          return undefined
        }
      })
    })
  })
}
