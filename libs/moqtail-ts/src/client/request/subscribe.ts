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
  FullTrackName,
  Location,
  MessageParameter,
  MoqtObject,
  Subscribe,
  RequestError,
  SubscribeOk,
  RequestUpdate,
  applyMessageParameterUpdate,
} from '@/model'
import { createLogger } from '../../util/logger'

const logger = createLogger('request/subscribe')

// TODO: Add timeout mechanism for unsubscribing
export class SubscribeRequest implements PromiseLike<SubscribeOk | RequestError> {
  requestId: bigint
  fullTrackName: FullTrackName
  isCanceled: boolean = false
  startLocation: Location | undefined
  endGroup: bigint | undefined
  priority: number
  forward: boolean
  subscribeParameters: MessageParameter[]
  largestLocation: Location | undefined // Updated on each received object
  streamsAccepted: bigint = 0n
  expectedStreams: bigint | undefined // Defined upon SUBSCRIBE_DONE
  readonly controller!: ReadableStreamDefaultController<MoqtObject>
  readonly stream: ReadableStream<MoqtObject>
  #promise: Promise<SubscribeOk | RequestError>
  #resolve!: (value: SubscribeOk | RequestError | PromiseLike<SubscribeOk | RequestError>) => void
  #reject!: (reason?: any) => void

  constructor(msg: Subscribe) {
    this.requestId = msg.requestId
    this.fullTrackName = msg.fullTrackName
    const filter = msg.parameters.find(MessageParameter.isSubscriptionFilter)
    this.startLocation = filter?.startLocation
    this.endGroup = filter?.endGroup
    const subPriority = msg.parameters.find(MessageParameter.isSubscriberPriority)
    this.priority = subPriority?.priority ?? 128
    const fwd = msg.parameters.find(MessageParameter.isForward)
    this.forward = fwd?.forward ?? true
    this.subscribeParameters = msg.parameters
    this.stream = new ReadableStream<MoqtObject>({
      start: (controller) => {
        ;(this.controller as any) = controller
      },
    })
    this.#promise = new Promise<SubscribeOk | RequestError>((resolve, reject) => {
      this.#resolve = resolve
      this.#reject = reject
    })
    logger.debug(
      `created requestId=${this.requestId} ftn="${this.fullTrackName}" priority=${this.priority} forward=${this.forward}`,
    )
  }
  update(msg: RequestUpdate): void {
    const filter = msg.parameters.find(MessageParameter.isSubscriptionFilter)
    if (filter?.startLocation !== undefined) this.startLocation = filter.startLocation
    if (filter?.endGroup !== undefined) this.endGroup = filter.endGroup

    for (const param of msg.parameters) {
      if (MessageParameter.isSubscriberPriority?.(param)) {
        this.priority = (param as any).priority
      } else if (MessageParameter.isForward?.(param)) {
        this.forward = (param as any).forward
      }
    }

    if (typeof applyMessageParameterUpdate === 'function') {
      applyMessageParameterUpdate(this.subscribeParameters, msg.parameters)
    }
  }
  switch(newTrackName: FullTrackName, newParameters: MessageParameter[]): void {
    this.fullTrackName = newTrackName
    this.subscribeParameters = newParameters
    this.#promise = new Promise<SubscribeOk | RequestError>((resolve, reject) => {
      this.#resolve = resolve
      this.#reject = reject
    })
  }
  unsubscribe(): void {
    this.isCanceled = true
  }
  resolve(value: SubscribeOk | RequestError | PromiseLike<SubscribeOk | RequestError>): void {
    if (value instanceof RequestError) {
      logger.error(
        `resolved with error requestId=${this.requestId} code=${value.errorCode} reason="${value.reasonPhrase.phrase}"`,
      )
    } else if (value instanceof SubscribeOk) {
      logger.debug(`resolved with OK requestId=${this.requestId} trackAlias=${value.trackAlias}`)
    }
    this.#resolve(value)
  }

  reject(reason?: any): void {
    logger.error(`rejected requestId=${this.requestId}`, reason)
    this.#reject(reason)
  }

  then<TResult1 = SubscribeOk | RequestError, TResult2 = never>(
    onfulfilled?: ((value: SubscribeOk | RequestError) => TResult1 | PromiseLike<TResult1>) | undefined | null,
    onrejected?: ((reason: any) => TResult2 | PromiseLike<TResult2>) | undefined | null,
  ): PromiseLike<TResult1 | TResult2> {
    return this.#promise.then(onfulfilled, onrejected)
  }

  catch<TResult = never>(
    onrejected?: ((reason: any) => TResult | PromiseLike<TResult>) | undefined | null,
  ): Promise<SubscribeOk | RequestError | TResult> {
    return this.#promise.catch(onrejected)
  }

  finally(onfinally?: (() => void) | undefined | null): Promise<SubscribeOk | RequestError> {
    return this.#promise.finally(onfinally)
  }
}
