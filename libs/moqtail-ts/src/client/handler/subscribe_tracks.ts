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

import { assertWithinFilterRangeLimit } from '@/client/util/validators'
import { RequestErrorCode } from '@/model/constants'
import { requestError, requestOk } from '@/model/messages'
import type { SubscribeTracks } from '@/model/messages'
import { InvalidFilterError, carriesRangeFilter } from '@/model/parameters'
import type { RequestStreamMessageHandler } from './handler'

export const handlerSubscribeTracks: RequestStreamMessageHandler<SubscribeTracks> = async (client, msg, stream) => {
  // Which tracks match is the application's call, made in a `peersubscribetracks`
  // listener, so a filter here could not be honoured. One within the limit is refused as
  // unsupported rather than accepted and ignored.
  if (carriesRangeFilter(msg.parameters)) {
    await stream.send(filterRefusal(msg, client.maxFilterRanges))
    return
  }

  client.emit('peersubscribetracks', msg)
  // The peer holds this stream open for the subscription's lifetime. Each matching track
  // is pushed as a PUBLISH on its own request stream, not on this one.
  await stream.send(requestOk())
}

function filterRefusal(msg: SubscribeTracks, limit: bigint) {
  try {
    assertWithinFilterRangeLimit(msg.parameters, limit, 'SUBSCRIBE_TRACKS')
  } catch (error) {
    if (!(error instanceof InvalidFilterError)) throw error

    return requestError(RequestErrorCode.InvalidFilter, error.details)
  }

  return requestError(RequestErrorCode.NotSupported, 'Range filters on SUBSCRIBE_TRACKS are not supported')
}
