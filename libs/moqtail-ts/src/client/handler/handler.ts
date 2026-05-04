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
  handlerMaxRequestId,
  handlerRequestsBlocked,
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
  MaxRequestId,
  RequestsBlocked,
  Subscribe,
  PublishDone,
  SubscribeOk,
  RequestUpdate,
  TrackStatus,
  RequestOk,
  RequestError,
  Unsubscribe,
  UnsubscribeNamespace,
  PublishOk,
} from '../../model/control'
import { MOQtailClient } from '../client'
import { ControlMessage } from '../../model/control'

export type ControlMessageHandler<T> = (client: MOQtailClient, msg: T) => Promise<void>

export function getHandlerForControlMessage(msg: ControlMessage): ControlMessageHandler<any> | undefined {
  if (msg instanceof Publish) return handlerPublish
  if (msg instanceof PublishOk) return handlerPublishOk
  if (msg instanceof PublishDone) return handlerPublishDone
  if (msg instanceof PublishNamespace) return handlerPublishNamespace
  if (msg instanceof PublishNamespaceCancel) return handlerPublishNamespaceCancel
  if (msg instanceof PublishNamespaceDone) return handlerPublishNamespaceDone
  if (msg instanceof Fetch) return handlerFetch
  if (msg instanceof FetchCancel) return handlerFetchCancel
  if (msg instanceof FetchOk) return handlerFetchOk
  if (msg instanceof GoAway) return handlerGoAway
  if (msg instanceof MaxRequestId) return handlerMaxRequestId
  if (msg instanceof Subscribe) return handlerSubscribe
  if (msg instanceof SubscribeOk) return handlerSubscribeOk
  if (msg instanceof RequestUpdate) return handlerSubscribeUpdate
  if (msg instanceof RequestsBlocked) return handlerRequestsBlocked
  if (msg instanceof TrackStatus) return handlerTrackStatus
  if (msg instanceof RequestOk) return handlerRequestOk
  if (msg instanceof RequestError) return handlerRequestError
  if (msg instanceof Unsubscribe) return handlerUnsubscribe
  if (msg instanceof UnsubscribeNamespace) return handlerUnsubscribeNamespace
  return undefined
}
