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

import { ProtocolViolationError } from '@/model'
import { SubscribeOk } from '../../model/control'
import { SubscribeRequest } from '../request/subscribe'
import { RequestStreamMessageHandler } from './handler'
import { logger } from '../../util/logger'
import { applySubscriptionResponse } from './subscription_response'

export const handlerSubscribeOk: RequestStreamMessageHandler<SubscribeOk> = async (client, msg, _stream, requestId) => {
  logger.debug('handler/subscribe_ok', `received requestId=${requestId} trackAlias=${msg.trackAlias}`)

  // SUBSCRIBE_OK carries no request id of its own: the stream it arrived on names the
  // SUBSCRIBE it answers (§10.1).
  const request = client.requests.get(requestId)
  if (request instanceof SubscribeRequest) {
    applySubscriptionResponse(client, request, msg.parameters, msg.trackProperties)
    logger.debug(
      'handler/subscribe_ok',
      `requestId=${requestId} — resolving SubscribeRequest ftn="${request.fullTrackName}"`,
    )
    request.resolve(msg)
  } else {
    logger.error(
      'handler/subscribe_ok',
      `requestId=${requestId} — no pending SubscribeRequest found (got ${request?.constructor.name ?? 'undefined'})`,
    )
    throw new ProtocolViolationError('handlerSubscribeOk', 'SUBSCRIBE_OK on a stream with no pending subscribe request')
  }
}
