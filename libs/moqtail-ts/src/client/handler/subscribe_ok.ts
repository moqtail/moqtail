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
import { ControlMessageHandler } from './handler'
import { createLogger } from '../../util/logger'

const logger = createLogger('handler/subscribe_ok')

export const handlerSubscribeOk: ControlMessageHandler<SubscribeOk> = async (client, msg) => {
  logger.debug(`received requestId=${msg.requestId} trackAlias=${msg.trackAlias}`)

  const request = client.requests.get(msg.requestId)
  if (request instanceof SubscribeRequest) {
    if (msg.trackExtensions.length > 0) {
      logger.debug(`requestId=${msg.requestId} — applying ${msg.trackExtensions.length} track extension(s)`)
      const track = client.trackSources.get(request.fullTrackName.toString())
      if (track !== undefined) {
        track.trackExtensions = msg.trackExtensions
      }
    }
    logger.debug(`requestId=${msg.requestId} — resolving SubscribeRequest ftn="${request.fullTrackName}"`)
    request.resolve(msg)
  } else {
    logger.error(
      `requestId=${msg.requestId} — no pending SubscribeRequest found (got ${request?.constructor.name ?? 'undefined'})`,
    )
    throw new ProtocolViolationError('handlerSubscribeOk', 'No subscribe request was found with the given request id')
  }
}
