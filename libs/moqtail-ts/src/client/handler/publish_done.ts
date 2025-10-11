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
import { PublishDone } from '../../model/control'
import { ControlMessageHandler } from './handler'
import { SubscribeRequest } from '../request/subscribe'

export const handlerPublishDone: ControlMessageHandler<PublishDone> = async (client, msg) => {
  const request = client.requests.get(msg.requestId)
  if (request instanceof SubscribeRequest) {
    request.expectedStreams = msg.streamCount
  } else {
    throw new ProtocolViolationError('handlerPublishDone', 'No publish request was found with the given request id')
  }
}
