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

import { ProtocolViolationError } from '@/model/error'
import { FetchOk } from '../../model/control'
import { FetchRequest } from '../request/fetch'
import { SubscribeRequest } from '../request/subscribe'
import { RequestStreamMessageHandler } from './handler'
import { logger } from '../../util/logger'

export const handlerFetchOk: RequestStreamMessageHandler<FetchOk> = async (client, msg, _stream, requestId) => {
  logger.log('handler/fetch_ok', 'requestId', requestId)
  // FETCH_OK carries no request id of its own: the stream it arrived on names the FETCH
  // it answers (§10.1).
  const request = client.requests.get(requestId)
  if (request instanceof FetchRequest) {
    if (msg.trackProperties.length > 0) {
      const track = client.trackSources.get(request.message.fullTrackName.toString())
      if (track !== undefined) {
        track.trackProperties = msg.trackProperties
      }
    }
    request.resolve(msg)
  } else {
    throw new ProtocolViolationError(
      'handlerFetchOk',
      `FETCH_OK on a stream with no pending fetch request (request id ${requestId})`,
    )
  }
}
