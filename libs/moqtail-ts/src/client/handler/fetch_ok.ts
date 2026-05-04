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
import { FetchOk, FetchType } from '../../model/control'
import { FetchRequest } from '../request/fetch'
import { SubscribeRequest } from '../request/subscribe'
import { ControlMessageHandler } from './handler'
import { logger } from '../../util/logger'

export const handlerFetchOk: ControlMessageHandler<FetchOk> = async (client, msg) => {
  logger.log('handler/fetch_ok', 'requestId', msg.requestId)
  const request = client.requests.get(msg.requestId)
  if (request instanceof FetchRequest) {
    if (msg.trackExtensions.length > 0) {
      const fetchMsg = request.message
      let fullTrackName =
        fetchMsg.typeAndProps.type === FetchType.Standalone
          ? fetchMsg.typeAndProps.props.fullTrackName
          : (() => {
              const joiningReq = client.requests.get(fetchMsg.typeAndProps.props.joiningRequestId)
              return joiningReq instanceof SubscribeRequest ? joiningReq.fullTrackName : undefined
            })()
      if (fullTrackName !== undefined) {
        const track = client.trackSources.get(fullTrackName.toString())
        if (track !== undefined) {
          track.trackExtensions = msg.trackExtensions
        }
      }
    }
    request.resolve(msg)
  } else {
    throw new ProtocolViolationError(
      'handlerFetchOk',
      `No fetch request was found with the given request id: ${msg.requestId}`,
    )
  }
}
