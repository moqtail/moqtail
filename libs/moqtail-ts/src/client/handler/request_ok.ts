/**
 * Copyright 2026 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { RequestOk } from '../../model/control'
import { RequestStreamMessageHandler } from './handler'
import { logger } from '../../util/logger'
import { ProtocolViolationError } from '@/model'
import { PublishRequest } from '../request/publish'
import { PublishNamespaceRequest } from '../request/publish_namespace'
import { SubscribeNamespaceRequest } from '../request/subscribe_namespace'
import { TrackStatusRequest } from '../request/track_status'
import { PublishPublication } from '../publication/publish'

export const handlerRequestOk: RequestStreamMessageHandler<RequestOk> = async (client, msg, _stream, requestId) => {
  logger.log('handler/request_ok', 'received RequestOk for requestId:', requestId)

  // REQUEST_OK carries no request id of its own: the stream it arrived on names the
  // request it answers (§10.1).
  const request = client.requests.get(requestId)

  if (!request) {
    logger.warn('handler/request_ok', `Received RequestOk for unknown or already-resolved request id: ${requestId}`)
    return
  }

  msg.validateTrackProperties(request instanceof TrackStatusRequest)

  if (request instanceof PublishRequest) {
    request.resolve(msg)

    const fullTrackName = client.requestIdMap.getNameByRequestId(requestId)
    if (!fullTrackName) {
      logger.warn('handler/request_ok', `No track mapped for PublishRequest requestId: ${requestId}`)
      return
    }

    const track = client.trackSources.get(fullTrackName.toString())
    if (!track || !track.trackSource.live) {
      logger.warn('handler/request_ok', `Live track source not found for ${fullTrackName.toString()}`)
      return
    }

    client.publications.set(requestId, new PublishPublication(client, track, request.message))
    return
  }

  if (
    request instanceof PublishNamespaceRequest ||
    request instanceof SubscribeNamespaceRequest ||
    request instanceof TrackStatusRequest
  ) {
    // Resolve the promise so the awaiting client code can continue
    request.resolve(msg)

    // remove the request from the map
    client.requests.delete(requestId)
  } else {
    // If the ID matches a request like 'FetchRequest' which expects a 'FetchOk',
    // the server sent us the wrong message type!
    throw new ProtocolViolationError(
      'handlerRequestOk',
      `Request ID ${requestId} matched a pending request, but it does not expect a RequestOk response.`,
    )
  }
}
