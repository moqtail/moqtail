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
import { ControlMessageHandler } from './handler'
import { createLogger } from '../../util/logger'
import { ProtocolViolationError } from '@/model'
import { PublishNamespaceRequest } from '../request/publish_namespace'
import { SubscribeNamespaceRequest } from '../request/subscribe_namespace'
import { TrackStatusRequest } from '../request/track_status'

const logger = createLogger('handler/request_ok')

export const handlerRequestOk: ControlMessageHandler<RequestOk> = async (client, msg) => {
  logger.log('received RequestOk for requestId:', msg.requestId)

  // 1. Look up the pending request by ID
  const request = client.requests.get(msg.requestId)

  // 2. Handle untracked IDs gracefully
  if (!request) {
    logger.warn(`Received RequestOk for unknown or already-resolved request id: ${msg.requestId}`)
    return
  }

  // 3. Verify that the pending request actually expects a RequestOk response
  if (
    request instanceof PublishNamespaceRequest ||
    request instanceof SubscribeNamespaceRequest ||
    request instanceof TrackStatusRequest
  ) {
    // Resolve the promise so the awaiting client code can continue
    request.resolve(msg)

    // remove the request from the map
    client.requests.delete(msg.requestId)
  } else {
    // If the ID matches a request like 'FetchRequest' which expects a 'FetchOk',
    // the server sent us the wrong message type!
    throw new ProtocolViolationError(
      'handlerRequestOk',
      `Request ID ${msg.requestId} matched a pending request, but it does not expect a RequestOk response.`,
    )
  }
}
