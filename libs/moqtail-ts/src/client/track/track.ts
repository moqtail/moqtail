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

import { FullTrackName, ObjectForwardingPreference } from '@/model'
import { TrackSource } from './content_source'
import { TrackExtension } from '../../model/extension_header/track_extension'

/**
 * Describes a media/data track known to the client (either published locally or subscribed to).
 *
 *
 * @example Live only track
 * ```ts
 * const liveStream: ReadableStream<MoqtObject> = buildCameraStream()
 * const track: Track = {
 *   fullTrackName,
 *   forwardingPreference: ObjectForwardingPreference.Latest,
 *   trackSource: { live: liveStream },
 *   publisherPriority: 0
 * }
 * client.addOrUpdateTrack(track)
 * ```
 *
 * @example Past only track (pre‑recorded cache)
 * ```ts
 * const cache = new MemoryObjectCache()
 * recording.forEach(obj => cache.add(obj))
 * const track: Track = {
 *   fullTrackName,
 *   forwardingPreference: ObjectForwardingPreference.All,
 *   trackSource: { past: cache },
 *   publisherPriority: 64
 * }
 * client.addOrUpdateTrack(track)
 * ```
 *
 * @example Hybrid (cache + live)
 * ```ts
 * const cache = new MemoryObjectCache()
 * const liveStream = buildLiveReadableStream()
 * const track: Track = {
 *   fullTrackName,
 *   forwardingPreference: ObjectForwardingPreference.Latest,
 *   trackSource: { past: cache, live: liveStream },
 *   publisherPriority: 8
 * }
 * client.addOrUpdateTrack(track)
 * ```
 */
export type Track = {
  /**
   * Globally unique identifier for the track.
   */
  fullTrackName: FullTrackName

  /**
   * Hint controlling which objects SHOULD be forwarded / prioritized.
   */
  forwardingPreference: ObjectForwardingPreference

  /**
   * Accessors for live and/or past objects belonging to this track.
   */
  trackSource: TrackSource

  /**
   * 0 (highest) .. 255 (lowest) priority advertised with objects.
   * Values are rounded to nearest integer then clamped between 0 and 255.
   */
  publisherPriority: number

  /**
   * Optional compact numeric alias assigned during protocol negotiation.
   */
  trackAlias?: bigint

  /**
   * Track extensions advertised by the publisher.
   * Set before calling `publish()` to include extensions in the PUBLISH message.
   * Populated automatically on the subscriber side when SUBSCRIBE_OK or FETCH_OK is received.
   */
  trackExtensions?: TrackExtension[]
}
