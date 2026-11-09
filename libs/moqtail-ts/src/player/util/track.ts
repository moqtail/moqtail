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

/** Identity and bitrate of a catalog track, shared by every source element. */

import type { CatalogReader, CatalogTrack, Rendition } from '@/player/element/interfaces'
import { formatMsfUrl, trackIdentifier } from '@/player/msf_url'

/** Namespace and name, encoded as in MSF 11.1.2, so tracks compare as bytes (MSF 11.2.2). */
export function trackKey(track: CatalogTrack): string {
  return trackIdentifier(track.namespace, track.nameBytes)
}

/** The track name, prefixed with its namespace when it differs from the catalog's. */
export function renditionId(track: CatalogTrack, catalog: CatalogReader | undefined): string {
  return catalog && !track.namespace.equals(catalog.namespace) ? trackKey(track) : track.name
}

export function bitrateOf(track: CatalogTrack): number {
  return track.avgBitrate ?? track.bitrate ?? 0
}

export function rendition(
  track: CatalogTrack,
  catalog: CatalogReader | undefined,
  state: { active: boolean; selected: boolean },
): Rendition {
  return {
    id: renditionId(track, catalog),
    src: formatMsfUrl(catalog?.url ?? '', track.namespace, track.nameBytes),
    bitrate: bitrateOf(track),
    codec: track.codec ?? '',
    ...(track.width !== undefined && { width: track.width }),
    ...(track.height !== undefined && { height: track.height }),
    ...(track.framerate !== undefined && { frameRate: track.framerate }),
    ...state,
  }
}
