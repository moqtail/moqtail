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
  CaptureTimestamp = 2,
  VideoFrameMarking = 4,
  AudioLevel = 6,
  VideoConfig = 13,
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
    case 2:
      return LOCHeaderExtensionId.CaptureTimestamp
    case 4:
      return LOCHeaderExtensionId.VideoFrameMarking
    case 6:
      return LOCHeaderExtensionId.AudioLevel
    case 13:
      return LOCHeaderExtensionId.VideoConfig
    default:
      throw new InvalidTypeError('locHeaderExtensionIdFromNumber', `Invalid LOC header extension id: ${value}`)
  }
}
