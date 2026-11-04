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

/**
 * Where a track's CMAF init data comes from (MSF 5.1.7, 12.2): inline in the catalog, a
 * track property in SUBSCRIBE_OK or FETCH_OK, or an object property on the media objects.
 */

import type { KeyValuePair } from '@/model/common/pair'
import { AudioConfig } from '@/model/property/audio_config'
import { findProperty, type PropertyReader } from '@/model/property/find'
import type { TrackProperty } from '@/model/property/track_property'
import { VideoConfig } from '@/model/property/video_config'
import type { CatalogTrack } from '@/player/element/interfaces'

// VIDEO_CONFIG 0x0D and AUDIO_CONFIG 0x0F (moqt-22 Table 16; MSF 12.2 misspells the latter).
const READERS = new Map<number, PropertyReader<{ config: Uint8Array }>>([
  [VideoConfig.TYPE, VideoConfig],
  [AudioConfig.TYPE, AudioConfig],
])

function readerFor(property: number): PropertyReader<{ config: Uint8Array }> {
  const reader = READERS.get(property)
  if (!reader) throw new Error(`Init data in property 0x${property.toString(16)} isn't supported`)
  return reader
}

/**
 * Init data known once a request is accepted: inline, or the track property. Undefined
 * for tracks without init data, and for object-property tracks, which carry it on objects.
 * @throws Error when the catalog names an unknown property, or a track property that isn't there.
 */
export function acceptedInitData(track: CatalogTrack, trackProperties: TrackProperty[]): Uint8Array | undefined {
  const init = track.init
  if (!init) return undefined
  if (init.type === 'inline') return init.data
  const reader = readerFor(init.property)
  if (init.type === 'object-property') return undefined
  const data = findProperty(trackProperties, reader)?.config
  if (!data) throw new Error(`Track property 0x${init.property.toString(16)} with the init data is missing`)
  return data
}

/** The init data an object of an object-property track carries, if any. */
export function objectInitData(track: CatalogTrack, properties: KeyValuePair[] | null): Uint8Array | undefined {
  if (track.init?.type !== 'object-property') return undefined
  return findProperty(properties, readerFor(track.init.property))?.config
}
