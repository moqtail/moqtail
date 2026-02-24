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

import { ProtocolViolationError } from '@/model'
import { PublishOk } from '../../model/control'
import { PublishRequest } from '../request/publish'
import { ControlMessageHandler } from './handler'
import { PublishPublication } from '../publication/publish'

export const handlerPublishOk: ControlMessageHandler<PublishOk> = async (client, msg) => {
  const request = client.requests.get(msg.requestId)
  if (request instanceof PublishRequest) {
    request.resolve(msg)
    const fullTrackName = client.requestIdMap.getNameByRequestId(msg.requestId)
    if (!fullTrackName) {
      console.warn(`[MOQtail] No track mapped for PublishOk requestId: ${msg.requestId}`)
      return
    }

    const track = client.trackSources.get(fullTrackName.toString())
    if (!track || !track.trackSource.live) {
      console.warn(`[MOQtail] Live track source not found for ${fullTrackName.toString()}`)
      return
    }

    const publication = new PublishPublication(client, track, request.message)

    client.publications.set(msg.requestId, publication)
  } else {
    throw new ProtocolViolationError('handlerPublishOk', 'No publish request was found with the given request id')
  }
}
