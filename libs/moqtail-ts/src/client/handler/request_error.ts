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

import { RequestError } from '../../model/control'
import { ControlMessageHandler } from './handler'
import { createLogger } from '../../util/logger'
import { SubscribeRequest } from '../request/subscribe'
import { PublishRequest } from '../request/publish'
import { PublishNamespaceRequest } from '../request/publish_namespace'
import { SubscribeNamespaceRequest } from '../request/subscribe_namespace'
import { FetchRequest } from '../request/fetch'

const logger = createLogger('handler/request_error')

export const handlerRequestError: ControlMessageHandler<RequestError> = async (client, msg) => {
  logger.warn('requestId, code, reason', msg.requestId, msg.errorCode, msg.reasonPhrase)

  const request = client.requests.get(msg.requestId)
  if (!request) {
    logger.warn(`Received RequestError for unknown or already-resolved request id: ${msg.requestId}`)
    return
  }

  if (
    request instanceof SubscribeRequest ||
    request instanceof PublishRequest ||
    request instanceof PublishNamespaceRequest ||
    request instanceof SubscribeNamespaceRequest ||
    request instanceof FetchRequest
  ) {
    request.resolve(msg)
    client.requests.delete(msg.requestId)
  }
}
