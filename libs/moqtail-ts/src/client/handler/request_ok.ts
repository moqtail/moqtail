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

import type { MOQtailClient } from '@/client/client'
import { PublishPublication } from '@/client/publication/publish'
import { FetchRequest } from '@/client/request/fetch'
import { SubscribeRequest } from '@/client/request/subscribe'
import { ProtocolViolationError } from '@/model/error/error'
import type { Publish, RequestOk } from '@/model/messages'
import { logger } from '@/util/logger'
import type { RequestStreamMessageHandler } from './handler'
import { applySubscriptionResponse } from './subscription_response'

export const handlerRequestOk: RequestStreamMessageHandler<RequestOk> = async (client, msg, _stream, requestId) => {
  // REQUEST_OK carries no request id of its own: the stream it arrived on names the
  // request it answers (§10.1).
  const request = client.requests.get(requestId)
  if (!request) {
    logger.warn('handler/request_ok', `REQUEST_OK for unknown or already-resolved request id ${requestId}`)
    return
  }

  if (request instanceof FetchRequest) {
    throw new ProtocolViolationError('handlerRequestOk', `request ${requestId} expects a FETCH_OK, not a REQUEST_OK`)
  }
  if (msg.trackProperties.length > 0 && request.message.kind !== 'trackStatus') {
    throw new ProtocolViolationError('handlerRequestOk', 'Track Properties present in a non-TRACK_STATUS REQUEST_OK')
  }

  // A REQUEST_UPDATE on a live subscription is answered with REQUEST_OK on its own
  // stream. The subscription carries on; what the response adds is where a fill it asked
  // for stops, and how long the subscription is good for.
  if (request instanceof SubscribeRequest) {
    applySubscriptionResponse(client, request, msg)
    return
  }

  request.resolve(msg)

  if (request.message.kind === 'publish') {
    startPublication(client, requestId, request.message)
    return
  }

  client.requests.delete(requestId)
}

/**
 * The publication starts at the Forward State the PUBLISH declared. A peer that wants
 * something else says so in a REQUEST_UPDATE after this PUBLISH_OK.
 */
function startPublication(client: MOQtailClient, requestId: bigint, msg: Publish): void {
  const track = client.trackSources.get(msg.fullTrackName.toString())
  if (!track?.trackSource.live) {
    logger.warn('handler/request_ok', `Live track source not found for ${msg.fullTrackName}`)
    return
  }

  client.publications.set(requestId, new PublishPublication(client, track, msg))
}
