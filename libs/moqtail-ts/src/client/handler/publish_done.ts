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

import { SubscribeRequest } from '@/client/request/subscribe'
import type { PublishDone } from '@/model/messages'
import type { RequestStreamMessageHandler } from './handler'

export const handlerPublishDone: RequestStreamMessageHandler<PublishDone> = async (
  client,
  msg,
  _stream,
  openingRequestId,
) => {
  client.emit('peerpublishdone', msg, openingRequestId)

  // A track the peer pushed with PUBLISH has no request of ours; its receiver is held under its alias
  const alias = client.subscriptionAliasMap.get(openingRequestId)
  const found = client.requests.get(openingRequestId)
  const request =
    found instanceof SubscribeRequest
      ? found
      : found === undefined && alias !== undefined
        ? client.subscriptions.get(alias)
        : undefined
  // TODO: a PUBLISH_DONE for no request is a protocol violation, once the relay stops sending them
  if (!request) return

  request.expectedStreams = msg.streamCount
  request.publishDone = msg
  // The streams it accounts for may all have arrived already, in which case this
  // message is the last thing the subscription was waiting for.
  client.completeSubscription(request)
}
