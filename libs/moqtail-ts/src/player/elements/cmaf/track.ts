/**
 * Copyright 2026 The MOQtail Authors
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

/** What a catalog track means to the CMAF source: MIME type and pad capabilities. */

import { CapabilityCode, type Capabilities } from '@/player/element/define'
import type { CatalogTrack } from '@/player/element/interfaces'

/** `mimeType` (or `<role>/mp4`) with the codec, unless it already names one. */
export function mimeTypeOf(track: Partial<CatalogTrack>): string {
  const base = track.mimeType ?? `${track.role}/mp4`
  return base.includes('codecs=') || !track.codec ? base : `${base}; codecs="${track.codec}"`
}

/** A pad carrying a `video` or `audio` CMAF set. */
export function cmafCapabilities(direction: 'in' | 'out', role: string): Capabilities {
  return [
    { direction, code: CapabilityCode.PACKAGING, value: 'cmaf' },
    { direction, code: CapabilityCode.ROLE, value: role },
  ]
}
