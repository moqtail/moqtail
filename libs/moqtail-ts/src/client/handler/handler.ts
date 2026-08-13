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
  handlerSubscribe,
  handlerSubscribeOk,
  handlerSubscribeUpdate,
  handlerTrackStatus,
  handlerRequestOk,
  handlerRequestError,
  handlerFetch,
  handlerFetchOk,
  handlerGoAway,
  handlerGoAwayOnRequestStream,
  handlerPublish,
  handlerPublishBlocked,
  handlerSubscribeNamespace,
  handlerSubscribeTracks,
} from '.'
import {
  Publish,
  PublishBlocked,
  PublishNamespace,
  Fetch,
  FetchOk,
  GoAway,
  Subscribe,
  PublishDone,
  SubscribeOk,
  RequestUpdate,
  TrackStatus,
  RequestOk,
  RequestError,
  SubscribeNamespace,
  SubscribeTracks,
} from '../../model/control'
import { MOQtailClient } from '../client'
import { ControlMessage } from '../../model/control'
import { RequestStream } from '../request_stream'

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
  if (msg instanceof SubscribeTracks) return handlerSubscribeTracks
  // Responses, correlated by the stream they arrive on.
  if (msg instanceof SubscribeOk) return handlerSubscribeOk
  if (msg instanceof FetchOk) return handlerFetchOk
  // RequestOk also answers PUBLISH: PUBLISH_OK (0x1E) is an alias of REQUEST_OK, so
  // both codepoints arrive here as a RequestOk.
  if (msg instanceof RequestOk) return handlerRequestOk
  if (msg instanceof RequestError) return handlerRequestError
  // Follow-ups on an open request stream.
  // GOAWAY is Control *and* Request in Table 5: here it migrates this one request.
  if (msg instanceof GoAway) return handlerGoAwayOnRequestStream
  if (msg instanceof RequestUpdate) return handlerSubscribeUpdate
  if (msg instanceof PublishDone) return handlerPublishDone
  // The publisher's answer when the peer's bidi stream limit leaves it no stream to
  // send a track's PUBLISH on (§10.20). It travels on the SUBSCRIBE_TRACKS stream.
  if (msg instanceof PublishBlocked) return handlerPublishBlocked
  return undefined
}
