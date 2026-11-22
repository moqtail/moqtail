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

import { isSessionClosed } from '@/client/util/stream_reset'
import type {
  Publish,
  PublishNamespace,
  RequestError,
  RequestOk,
  SubscribeNamespace,
  TrackStatus,
} from '@/model/messages'
import { logger } from '@/util/logger'

/** The requests that need nothing beyond their response. */
export type RequestMessage = Publish | PublishNamespace | SubscribeNamespace | TrackStatus

/**
 * @public
 * A request this side sent, awaiting its response: awaiting it gives the response, or the
 * REQUEST_ERROR that refused it.
 */
export class PendingRequest<
  M extends { requestId: bigint } = RequestMessage,
  R = RequestOk | RequestError,
> implements PromiseLike<R> {
  readonly requestId: bigint
  readonly message: M
  readonly #response = Promise.withResolvers<R>()

  constructor(message: M) {
    this.requestId = message.requestId
    this.message = message
  }

  resolve(value: R): void {
    this.#response.resolve(value)
  }

  reject(reason?: unknown): void {
    if (!isSessionClosed(reason)) logger.error('request', `request ${this.requestId} failed`, reason)

    this.#response.reject(reason)
  }

  then<TResult1 = R, TResult2 = never>(
    onfulfilled?: ((value: R) => TResult1 | PromiseLike<TResult1>) | null,
    onrejected?: ((reason: unknown) => TResult2 | PromiseLike<TResult2>) | null,
  ): PromiseLike<TResult1 | TResult2> {
    return this.#response.promise.then(onfulfilled, onrejected)
  }
}
