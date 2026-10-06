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

import { InternalError, Location, ReasonPhrase } from '@/model'
import { Fetch, FetchOk, RequestError, RequestErrorCode } from '../../model/control'
import { RequestStreamMessageHandler } from './handler'
import { SubscribePublication } from '../publication/subscribe'
import { FetchPublication } from '../publication/fetch'
import { PublishPublication } from '../publication'
import { logger } from '../../util/logger'

export const handlerFetch: RequestStreamMessageHandler<Fetch> = async (client, msg, stream) => {
  logger.log('handler/fetch', 'requestId', msg.requestId)
  // TODO: Use fetch parameters and handle authorization
  const fullTrackName = msg.fullTrackName
  const track = client.trackSources.get(fullTrackName.toString())
  if (!track) {
    const response = new RequestError(RequestErrorCode.DoesNotExist, 0n, new ReasonPhrase('Track does not exists'))
    await stream.send(response)
    return
  }

  if (!track.trackSource.past) {
    const response = new RequestError(
      RequestErrorCode.NotSupported,
      0n,
      new ReasonPhrase('Requested track does not support fetch'),
    )
    await stream.send(response)
    return
  }
  // TODO: Add support for descending group order
  // TODO: Handle parameter checking and parameter selection.
  // TODO: Figure out what to do with endOfTrack and end location
  const publication = new FetchPublication(client, track, msg)
  client.publications.set(msg.requestId, publication)
  const response = new FetchOk(false, new Location(0n, 0n), msg.parameters, track.trackProperties ?? [])
  await stream.send(response)
}
