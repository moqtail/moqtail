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
import { Subscribe, SubscribeError, SubscribeErrorCode, SubscribeOk } from '../../model/control'
import { ControlMessageHandler } from './handler'
import { SubscribePublication } from '../publication/subscribe'
import { random60bitId } from '../util/random_id'

export const handlerSubscribe: ControlMessageHandler<Subscribe> = async (client, msg) => {
  const track = client.trackSources.get(msg.fullTrackName.toString())
  if (!track) {
    const subscribeError = new SubscribeError(
      msg.requestId,
      SubscribeErrorCode.TrackDoesNotExist,
      new ReasonPhrase('Track does not exist'),
      0n, // TODO: Since track does not exist alias is set to zero. This argument is removed in the upcoming version
    )
    await client.controlStream.send(subscribeError)
    return
  }
  if (!track.trackSource.live) {
    const response = new SubscribeError(
      msg.requestId,
      SubscribeErrorCode.NotSupported,
      new ReasonPhrase('Requested track does not support subscribe'),
      0n,
    )
    await client.controlStream.send(response)
    return
  }
  let subscribeOk: SubscribeOk
  if (track.trackSource.live.largestLocation) {
    subscribeOk = SubscribeOk.newAscendingWithContent(
      msg.requestId,
      0n,
      track.trackSource.live.largestLocation,
      msg.subscribeParameters,
    )
  } else {
    // TODO: Add support for descending group order
    subscribeOk = SubscribeOk.newAscendingNoContent(msg.requestId, 0n, msg.subscribeParameters)
  }
  const publication = new SubscribePublication(client, track, msg, subscribeOk.largestLocation)
  client.publications.set(msg.requestId, publication)
  await client.controlStream.send(subscribeOk)
}
