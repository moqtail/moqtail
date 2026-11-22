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

import { SubscribePublication } from '@/client/publication/subscribe'
import { isValidTrackAlias, rangeFilterRejection } from '@/client/util/validators'
import { RequestErrorCode } from '@/model/constants'
import { requestError } from '@/model/messages'
import type { Subscribe, SubscribeOk } from '@/model/messages'
import { logger } from '@/util/logger'
import type { RequestStreamMessageHandler } from './handler'

export const handlerSubscribe: RequestStreamMessageHandler<Subscribe> = async (client, msg, stream) => {
  const filterRejection = rangeFilterRejection(msg.parameters, client.maxFilterRanges)
  if (filterRejection) {
    logger.warn('handler/subscribe', `requestId=${msg.requestId} has filters this client cannot take`)
    await stream.send(filterRejection)
    return
  }

  const track = client.trackSources.get(msg.fullTrackName.toString())
  if (!track) {
    await stream.send(requestError(RequestErrorCode.DoesNotExist, 'Track does not exist'))
    return
  }
  if (!track.trackSource.live) {
    await stream.send(requestError(RequestErrorCode.NotSupported, 'Requested track does not support subscribe'))
    return
  }
  if (!isValidTrackAlias(track.trackAlias)) throw new Error('Expected track alias to be set')

  // The request's own parameters are not echoed: SUBSCRIBE_OK carries only what the
  // publisher has to say, and a request parameter is not allowed in it.
  const largestLocation = track.trackSource.live.largestLocation
  const subscribeOk: SubscribeOk = {
    kind: 'subscribeOk',
    trackAlias: track.trackAlias,
    parameters: largestLocation ? { largestObject: largestLocation } : {},
    trackProperties: msg.parameters.includeProperties === false ? [] : (track.trackProperties ?? []),
  }

  // The publication keeps the stream: PUBLISH_DONE and any later response for this
  // subscription must go back on the stream the SUBSCRIBE arrived on.
  const publication = new SubscribePublication(client, track, msg, stream, largestLocation)
  client.publications.set(msg.requestId, publication)

  await stream.send(subscribeOk)
}
