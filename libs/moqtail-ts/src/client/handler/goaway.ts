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

import { GoAway } from '../../model/control'
import { ProtocolViolationError } from '../../model/error/error'
import { ControlMessageHandler, RequestStreamMessageHandler } from './handler'
import { logger } from '../../util/logger'

export const handlerGoAway: ControlMessageHandler<GoAway> = async (client, msg) => {
  logger.log('handler/goaway', 'newSessionUri', msg.newSessionUri, 'timeout', msg.timeout, 'requestId', msg.requestId)
  if (client.goawayReceived) {
    await client.disconnect(
      new ProtocolViolationError('handler/goaway', 'A second GOAWAY arrived on the control stream'),
    )
    return
  }
  client.goawayReceived = true
  if (client.onGoaway) {
    client.onGoaway(msg)
  }
}

/**
 * GOAWAY on a request stream: the peer is migrating that one request (§10.4). The
 * request is re-issued on a fresh stream and the session, along with every other request
 * on it, carries on.
 */
export const handlerGoAwayOnRequestStream: RequestStreamMessageHandler<GoAway> = async (
  client,
  msg,
  _stream,
  requestId,
) => {
  logger.log('handler/goaway', `GOAWAY migrating request ${requestId}`, 'timeout', msg.timeout)
  await client.migrateRequest(requestId, msg)
}
