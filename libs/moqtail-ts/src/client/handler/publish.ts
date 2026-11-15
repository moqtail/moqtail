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
import { Publish, RequestOk } from '@/model/control'
import { FullTrackName, MoqtObject } from '@/model/data'
import { logger } from '@/util/logger'
import { ObjectStream } from '@/client/util/object_stream'
import { RequestStreamMessageHandler } from './handler'

/**
 * A receiver for a track the peer pushes with PUBLISH. It stands in for a SubscribeRequest
 * and its Subscription, so the data stream and datagram paths deliver to it unchanged.
 */
export function pushedTrackReceiver(requestId: bigint, fullTrackName: FullTrackName) {
  const objects = new ObjectStream()
  const receiver = {
    requestId,
    fullTrackName,
    streamsAccepted: 0,
    largestLocation: undefined,
    manager: {
      objects,
      requests: [],
      drop() {},
      deliver(_request: unknown, obj: MoqtObject) {
        objects.enqueue(obj)
        return true
      },
    },
  }
  return { receiver, stream: objects.readable }
}

export const handlerPublish: RequestStreamMessageHandler<Publish> = async (client, msg, stream) => {
  logger.log('handler/publish', 'track: %s alias: %d', msg.fullTrackName.toString(), msg.trackAlias)

  const localPseudoRequestId = client.allocatePseudoRequestId()

  // Registered before the push is accepted, since its data streams may follow at once
  client.requestIdMap.addMapping(localPseudoRequestId, msg.fullTrackName)
  client.subscriptionAliasMap.set(localPseudoRequestId, msg.trackAlias)
  client.aliasFullTrackNameMap.set(msg.trackAlias, msg.fullTrackName)

  const { receiver, stream: objects } = pushedTrackReceiver(localPseudoRequestId, msg.fullTrackName)

  // Without a receiver for the alias, its data streams are a protocol violation
  client.subscriptions.set(msg.trackAlias, receiver)

  client.emit('peerpublish', msg, objects)

  // Accept the push on the stream it arrived on. The publisher waits for this
  // before opening any data stream. PUBLISH is answered by REQUEST_OK: PUBLISH_OK is
  // just that message's name for this request type (§10.5), not a message of its own.
  await stream.send(new RequestOk())
}
