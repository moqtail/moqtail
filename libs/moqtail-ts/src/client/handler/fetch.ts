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

import { FetchPublication } from '@/client/publication/fetch'
import { rangeFilterRejection } from '@/client/util/validators'
import { RequestErrorCode } from '@/model/constants'
import { requestError } from '@/model/messages'
import type { Fetch } from '@/model/messages'
import { fetchRange, locationFilter } from '@/model/parameters'
import { LocationFilterType } from '@/model/constants'
import { logger } from '@/util/logger'
import type { RequestStreamMessageHandler } from './handler'

export const handlerFetch: RequestStreamMessageHandler<Fetch> = async (client, msg, stream) => {
  const filterRejection = rangeFilterRejection(msg.parameters, client.maxFilterRanges)
  if (filterRejection) {
    logger.warn('handler/fetch', `requestId=${msg.requestId} has filters this client cannot take`)
    await stream.send(filterRejection)
    return
  }

  // TODO: Use fetch parameters and handle authorization
  const track = client.trackSources.get(msg.fullTrackName.toString())
  if (!track) {
    await stream.send(requestError(RequestErrorCode.DoesNotExist, 'Track does not exists'))
    return
  }
  if (!track.trackSource.past) {
    await stream.send(requestError(RequestErrorCode.NotSupported, 'Requested track does not support fetch'))
    return
  }

  const largest = track.trackSource.live?.largestLocation ?? (await track.trackSource.past.getRange()).at(-1)?.location
  const filter = msg.parameters.locationFilter ?? locationFilter(LocationFilterType.None)
  const range = fetchRange(filter, largest)
  if (!range) {
    await stream.send(requestError(RequestErrorCode.InvalidRange, 'Nothing published in the requested range'))
    return
  }

  // TODO: Add support for descending group order, and decide what endOfTrack says
  const publication = new FetchPublication(client, track, msg, range.start, range.end)
  client.publications.set(msg.requestId, publication)

  // No parameters are defined for FETCH_OK, so the request's are not echoed.
  const trackProperties = msg.parameters.includeProperties === false ? [] : (track.trackProperties ?? [])

  await stream.send({ kind: 'fetchOk', endOfTrack: false, endLocation: range.end, parameters: {}, trackProperties })
}
