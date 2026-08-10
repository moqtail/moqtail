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

import {
  handlerPublishDone,
  handlerPublishNamespace,
  handlerPublishNamespaceCancel,
  handlerPublishNamespaceDone,
  handlerSubscribe,
  handlerSubscribeOk,
  handlerSubscribeUpdate,
  handlerTrackStatus,
  handlerRequestOk,
  handlerRequestError,
  handlerUnsubscribe,
  handlerUnsubscribeNamespace,
  handlerFetch,
  handlerFetchCancel,
  handlerFetchOk,
  handlerGoAway,
  handlerPublish,
  handlerPublishOk,
  handlerSubscribeNamespace,
} from '.'
import {
  Publish,
  PublishNamespace,
  PublishNamespaceCancel,
  PublishNamespaceDone,
  Fetch,
  FetchCancel,
  FetchOk,
  GoAway,
  Subscribe,
  PublishDone,
  SubscribeOk,
  RequestUpdate,
  TrackStatus,
  RequestOk,
  RequestError,
  Unsubscribe,
  UnsubscribeNamespace,
  SubscribeNamespace,
  PublishOk,
} from '../../model/control'
import { MOQtailClient } from '../client'
import { ControlMessage } from '../../model/control'
import { RequestStream } from '../request_stream'

/** A message on the shared control stream. Nothing here has a stream to reply on. */
export type ControlMessageHandler<T> = (client: MOQtailClient, msg: T) => Promise<void>

/**
 * A message on a bidirectional request stream. `stream` is the stream the message
 * arrived on and the only place a response to it may be written.
 */
export type RequestStreamMessageHandler<T> = (client: MOQtailClient, msg: T, stream: RequestStream) => Promise<void>

/**
 * Draft-18 §3.3: the control stream carries SETUP and GOAWAY, nothing else. SETUP is
 * consumed by the handshake in {@link MOQtailClient.new}, so GOAWAY is all that reaches
 * the read loop. Every request type has moved to its own bidi stream — see
 * {@link getHandlerForRequestStreamMessage}.
 */
export function getHandlerForControlMessage(msg: ControlMessage): ControlMessageHandler<any> | undefined {
  if (msg instanceof GoAway) return handlerGoAway
  return undefined
}

/**
 * Handlers for everything that travels on a bidirectional request stream: the seven
 * `First`-marked types that open one, the responses that come back on it, and the
 * follow-ups either side may send while it is open.
 */
export function getHandlerForRequestStreamMessage(msg: ControlMessage): RequestStreamMessageHandler<any> | undefined {
  // Types that open a request stream (Table 5, `First`).
  if (msg instanceof Subscribe) return handlerSubscribe
  if (msg instanceof Fetch) return handlerFetch
  if (msg instanceof Publish) return handlerPublish
  if (msg instanceof PublishNamespace) return handlerPublishNamespace
  if (msg instanceof TrackStatus) return handlerTrackStatus
  if (msg instanceof SubscribeNamespace) return handlerSubscribeNamespace
  // Responses, correlated by the stream they arrive on.
  if (msg instanceof SubscribeOk) return handlerSubscribeOk
  if (msg instanceof FetchOk) return handlerFetchOk
  if (msg instanceof PublishOk) return handlerPublishOk
  if (msg instanceof RequestOk) return handlerRequestOk
  if (msg instanceof RequestError) return handlerRequestError
  // Follow-ups on an open request stream.
  if (msg instanceof RequestUpdate) return handlerSubscribeUpdate
  if (msg instanceof PublishDone) return handlerPublishDone
  if (msg instanceof PublishNamespaceDone) return handlerPublishNamespaceDone
  // Retired by draft-18; cancellation is a stream reset now. Kept until #261 deletes
  // the message types themselves so a peer still sending them is not fatal.
  if (msg instanceof Unsubscribe) return handlerUnsubscribe
  if (msg instanceof FetchCancel) return handlerFetchCancel
  if (msg instanceof PublishNamespaceCancel) return handlerPublishNamespaceCancel
  if (msg instanceof UnsubscribeNamespace) return handlerUnsubscribeNamespace
  return undefined
}
