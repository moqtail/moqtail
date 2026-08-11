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
import { Publish, RequestOk } from '../../model/control'
import { RequestStreamMessageHandler } from './handler'
import { MoqtObject } from '../../model/data' // Make sure to import MoqtObject
import { logger } from '../../util/logger'

export const handlerPublish: RequestStreamMessageHandler<Publish> = async (client, msg, stream) => {
  logger.log('handler/publish', 'track: %s alias: %d', msg.fullTrackName.toString(), msg.trackAlias)

  // 1. Create a stream to receive the pushed objects natively
  let streamController!: ReadableStreamDefaultController<MoqtObject>
  const objects = new ReadableStream<MoqtObject>({
    start(c) {
      streamController = c
    },
  })

  const localPseudoRequestId = client.allocatePseudoRequestId()

  // 2. Set up the expectation in the client BEFORE accepting the push
  client.requestIdMap.addMapping(localPseudoRequestId, msg.fullTrackName)
  client.subscriptionAliasMap.set(localPseudoRequestId, msg.trackAlias)
  client.aliasFullTrackNameMap.set(msg.trackAlias, msg.fullTrackName)

  // This object mimics a SubscribeRequest so #handleRecvStreams can use it identically
  const receiver = {
    requestId: localPseudoRequestId,
    streamsAccepted: 0,
    largestLocation: undefined,
    controller: streamController,
  }

  // 3. Register the receiver map so data streams don't trigger ProtocolViolationError
  client.subscriptions.set(msg.trackAlias, receiver)

  // 4. Bubble the event up to the application layer, passing the data stream!
  if (client.onPeerPublish) {
    client.onPeerPublish(msg, objects)
  }

  // 5. Accept the push on the stream it arrived on. The publisher waits for this
  // before opening any data stream. PUBLISH is answered by REQUEST_OK: PUBLISH_OK is
  // just that message's name for this request type (§10.5), not a message of its own.
  await stream.send(new RequestOk(msg.requestId))
}
