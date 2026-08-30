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

import { MessageParameter, TrackProperty } from '@/model'
import { MOQtailClient } from '../client'
import { SubscribeRequest } from '../request/subscribe'
import { logger } from '../../util/logger'

/**
 * Records what a SUBSCRIBE_OK or REQUEST_UPDATE_OK says about a subscription.
 * Both carry the same things, so both are read the same way.
 *
 * LARGEST_OBJECT is where a fill fetch stream stops, which is how a subscriber
 * tells a fill that finished from one that was cut short. Track properties, such
 * as whether the track supports dynamic groups, belong to the track rather than
 * to this subscription and are recorded there. EXPIRES is kept for callers to
 * read but nothing acts on it: no subscription expires on a timer here yet.
 */
export function applySubscriptionResponse(
  client: MOQtailClient,
  request: SubscribeRequest,
  parameters: MessageParameter[],
  trackProperties: TrackProperty[],
): void {
  const largest = parameters.find(MessageParameter.isLargestObject)
  if (largest) request.fillBoundary = largest.location

  const expires = parameters.find(MessageParameter.isExpires)
  if (expires) request.expires = expires.expires

  if (trackProperties.length > 0) {
    logger.debug(
      'handler/subscription_response',
      `requestId=${request.requestId} — applying ${trackProperties.length} track propert(ies)`,
    )
    const track = client.trackSources.get(request.fullTrackName.toString())
    if (track !== undefined) track.trackProperties = trackProperties
  }
}
