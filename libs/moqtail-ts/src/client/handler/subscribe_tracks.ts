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

import { SubscribeTracks, RequestOk, RequestError, RequestErrorCode } from '../../model/control'
import { MessageParameter, ReasonPhrase } from '../../model'
import { InvalidFilterError } from '../../model/parameter/message/range_filter'
import { RequestStreamMessageHandler } from './handler'
import { logger } from '../../util/logger'
import { assertWithinFilterRangeLimit } from '../util/validators'

export const handlerSubscribeTracks: RequestStreamMessageHandler<SubscribeTracks> = async (client, msg, stream) => {
  logger.log('handler/subscribe_tracks', 'prefix', msg.trackNamespacePrefix.toUtf8Path())
  // Which tracks match is the application's call, made in onPeerSubscribeTracks, so a
  // filter here could not be honoured. One within the limit is refused as unsupported
  // rather than accepted and ignored.
  if (MessageParameter.carriesRangeFilter(msg.parameters)) {
    let code = RequestErrorCode.NotSupported
    let reason = 'Range filters on SUBSCRIBE_TRACKS are not supported'
    try {
      assertWithinFilterRangeLimit(msg.parameters, client.maxFilterRanges, 'SUBSCRIBE_TRACKS')
    } catch (error) {
      if (!(error instanceof InvalidFilterError)) throw error
      code = RequestErrorCode.InvalidFilter
      reason = error.details
    }
    logger.warn('handler/subscribe_tracks', `requestId=${msg.requestId} refused: ${reason}`)
    await stream.send(new RequestError(code, 0n, new ReasonPhrase(reason)))
    return
  }

  if (client.onPeerSubscribeTracks) {
    client.onPeerSubscribeTracks(msg)
  }

  // The peer holds this stream open for the subscription's lifetime. Each matching
  // track is pushed as a PUBLISH on its own request stream, not on this one.
  await stream.send(new RequestOk())
}
