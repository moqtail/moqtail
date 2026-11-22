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

import type { MOQtailClient } from '@/client/client'
import type { RequestStream } from '@/client/request_stream'
import type { GoAway, Message, MessageKind } from '@/model/messages'
import { handlerFetch } from './fetch'
import { handlerFetchOk } from './fetch_ok'
import { handlerGoAway, handlerGoAwayOnRequestStream } from './goaway'
import { handlerPublish } from './publish'
import { handlerPublishDone } from './publish_done'
import { handlerPublishNamespace } from './publish_namespace'
import { handlerPublishSkipped } from './publish_skipped'
import { handlerRequestError } from './request_error'
import { handlerRequestOk } from './request_ok'
import { handlerRequestUpdate } from './request_update'
import { handlerSubscribe } from './subscribe'
import { handlerSubscribeNamespace } from './subscribe_namespace'
import { handlerSubscribeOk } from './subscribe_ok'
import { handlerSubscribeTracks } from './subscribe_tracks'
import { handlerTrackStatus } from './track_status'

/** A message on the shared control stream. Nothing here has a stream to reply on. */
export type ControlMessageHandler<T> = (client: MOQtailClient, msg: T) => Promise<void>

/**
 * A message on a bidirectional request stream. `stream` is the stream the message
 * arrived on and the only place a response to it may be written. `openingRequestId` is
 * the id of the request that opened it, which is what names the request for messages
 * that carry no id of their own.
 */
export type RequestStreamMessageHandler<T> = (
  client: MOQtailClient,
  msg: T,
  stream: RequestStream,
  openingRequestId: bigint,
) => Promise<void>

type RequestStreamHandlers = { [K in MessageKind]?: RequestStreamMessageHandler<Extract<Message, { kind: K }>> }

/**
 * Everything that travels on a bidirectional request stream: the seven `First`-marked
 * types that open one, the responses that come back on it, and the follow-ups either side
 * may send while it is open. REQUEST_OK also answers PUBLISH: PUBLISH_OK is its alias.
 */
const REQUEST_STREAM_HANDLERS: RequestStreamHandlers = {
  subscribe: handlerSubscribe,
  fetch: handlerFetch,
  publish: handlerPublish,
  publishNamespace: handlerPublishNamespace,
  trackStatus: handlerTrackStatus,
  subscribeNamespace: handlerSubscribeNamespace,
  subscribeTracks: handlerSubscribeTracks,
  subscribeOk: handlerSubscribeOk,
  fetchOk: handlerFetchOk,
  requestOk: handlerRequestOk,
  requestError: handlerRequestError,
  goAway: handlerGoAwayOnRequestStream,
  requestUpdate: handlerRequestUpdate,
  publishDone: handlerPublishDone,
  publishSkipped: handlerPublishSkipped,
}

/**
 * The control stream carries SETUP and GOAWAY, nothing else. SETUP is consumed by the
 * handshake in {@link MOQtailClient.init}, so GOAWAY is all that reaches the read loop.
 */
export function getHandlerForControlMessage(msg: Message): ControlMessageHandler<GoAway> | undefined {
  return msg.kind === 'goAway' ? handlerGoAway : undefined
}

export function getHandlerForRequestStreamMessage(msg: Message): RequestStreamMessageHandler<Message> | undefined {
  return REQUEST_STREAM_HANDLERS[msg.kind] as RequestStreamMessageHandler<Message> | undefined
}
