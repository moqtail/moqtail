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

import { ReasonPhrase } from '@/model'
import { Subscribe, RequestError, RequestErrorCode, SubscribeOk } from '../../model/control'
import { ControlMessageHandler } from './handler'
import { SubscribePublication } from '../publication/subscribe'
import { createLogger } from '../../util/logger'
import { LargestObject } from '../../model/parameter/message/largest_object'

const logger = createLogger('handler/subscribe')

export const handlerSubscribe: ControlMessageHandler<Subscribe> = async (client, msg) => {
  logger.log('requestId, trackName', msg.requestId, msg.fullTrackName.toString())
  const track = client.trackSources.get(msg.fullTrackName.toString())
  if (!track) {
    const subscribeError = new RequestError(
      msg.requestId,
      RequestErrorCode.DoesNotExist,
      0n,
      new ReasonPhrase('Track does not exist'),
    )
    await client.controlStream.send(subscribeError)
    return
  }
  if (!track.trackSource.live) {
    const response = new RequestError(
      msg.requestId,
      RequestErrorCode.NotSupported,
      0n,
      new ReasonPhrase('Requested track does not support subscribe'),
    )
    await client.controlStream.send(response)
    return
  }
  if (!track.trackAlias) throw new Error('Expected track alias to be set')

  const largestLocation = track.trackSource.live.largestLocation
  const parameters = [...msg.parameters]
  if (largestLocation) {
    parameters.push(new LargestObject(largestLocation))
  }

  const subscribeOk = SubscribeOk.create(msg.requestId, track.trackAlias, parameters, track.trackExtensions ?? [])
  const publication = new SubscribePublication(client, track, msg, largestLocation)
  client.publications.set(msg.requestId, publication)
  await client.controlStream.send(subscribeOk)
}
