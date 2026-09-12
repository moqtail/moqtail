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
import { DEFAULT_SWITCH_DRAIN_TIMEOUT_MS, type EarlyDiscardPolicyConfig } from '../types'
import { logger } from '../../util/logger'

// TODO: Add timeout mechanism for unsubscribing

/**
 * Represents an active subscription request in MOQT, implementing `PromiseLike`
 * for convenient async resolution when subscription setup succeeds or fails.
 */
export class SubscribeRequest implements PromiseLike<SubscribeOk | RequestError> {
  // Public Request Metadata & State
  public requestId: bigint
  public fullTrackName: FullTrackName
  public isCanceled: boolean = false
  public startLocation: Location | undefined
  public endGroup: bigint | undefined
  public priority: number
  public forward: boolean
  public subscribeParameters: MessageParameter[]
  public earlyDiscardPolicy: EarlyDiscardPolicyConfig | undefined

  // Track Data State
  public largestLocation: Location | undefined // Updated on each received object
  /**
   * The publisher's Largest Object as of the last SUBSCRIBE_OK or REQUEST_UPDATE_OK,
   * which is where a fill fetch stream stops.
   */
  public fillBoundary: Location | undefined
  /** EXPIRES from the last SUBSCRIBE_OK or REQUEST_UPDATE_OK, in milliseconds. */
  public expires: bigint | undefined
  public streamsAccepted: bigint = 0n
  public expectedStreams: bigint | undefined // Defined upon SUBSCRIBE_DONE

  /** The Subscription managing this request's output stream once switching is involved. */
  public manager?: Subscription

  // Stream & Controller Properties
  public readonly controller!: ReadableStreamDefaultController<MoqtObject>
  public readonly stream: ReadableStream<MoqtObject>

  // Deferred Promise State
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
      'request/subscribe',
      `created requestId=${this.requestId} ftn="${this.fullTrackName}" priority=${this.priority} forward=${this.forward}`,
    )
  }

  /**
   * Updates subscription parameters in-place when a `RequestUpdate` message is received.
   */
  public update(msg: RequestUpdate): void {
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

  /**
   * Marks this subscription request as canceled.
   */
  public unsubscribe(): void {
    this.isCanceled = true
  }

  /**
   * Resolves the internal promise with a `SubscribeOk` or `RequestError`.
   */
  public resolve(value: SubscribeOk | RequestError | PromiseLike<SubscribeOk | RequestError>): void {
    if (value instanceof RequestError) {
      logger.error(
        'request/subscribe',
        `resolved with error requestId=${this.requestId} code=${value.errorCode} reason="${value.reasonPhrase.phrase}"`,
      )
    } else if (value instanceof SubscribeOk) {
      logger.debug('request/subscribe', `resolved with OK requestId=${this.requestId} trackAlias=${value.trackAlias}`)
    }
    this.#resolve(value)
  }

  /**
   * Rejects the internal promise with an error/reason.
   */
  public reject(reason?: any): void {
    logger.error('request/subscribe', `rejected requestId=${this.requestId}`, reason)
    this.#reject(reason)
  }

  public then<TResult1 = SubscribeOk | RequestError, TResult2 = never>(
    onfulfilled?: ((value: SubscribeOk | RequestError) => TResult1 | PromiseLike<TResult1>) | undefined | null,
    onrejected?: ((reason: any) => TResult2 | PromiseLike<TResult2>) | undefined | null,
  ): PromiseLike<TResult1 | TResult2> {
    return this.#promise.then(onfulfilled, onrejected)
  }

  public catch<TResult = never>(
    onrejected?: ((reason: any) => TResult | PromiseLike<TResult>) | undefined | null,
  ): Promise<SubscribeOk | RequestError | TResult> {
    return this.#promise.catch(onrejected)
  }

  public finally(onfinally?: (() => void) | undefined | null): Promise<SubscribeOk | RequestError> {
    return this.#promise.finally(onfinally)
  }
}

/**
 * Owns the single consumer-facing stream for a SUBSCRIBE that may be switched to a
 * new track one or more times. A soft switch can keep producing from the old track
 * for an unknown time after SUBSCRIBE_OK, and a second switch may be issued before the
 * first target ever delivers anything, so only the first object actually delivered by
 * the newest tracked request is the guarantee that nothing older will send again.
 *
 * That first delivery is the handover, not the end of the old track: a soft switch
 * leaves it delivering up to the group the new one starts at, so the two meet without
 * a hole. Those objects keep coming through here until the publisher says the
 * subscription is done, or until it has been quiet long enough to say so itself.
 */
export class Subscription {
  public readonly controller: ReadableStreamDefaultController<MoqtObject>
  public readonly stream: ReadableStream<MoqtObject>

  /** Oldest first; the last entry is the newest switch target still racing to produce data. */
  #requests: SubscribeRequest[]

  /**
   * Requests a soft switch handed over from, still delivering the tail of what they
   * were told to finish, each with the timer that retires it if it goes quiet.
   */
  #draining: Map<SubscribeRequest, ReturnType<typeof setTimeout>> = new Map()

  /** Switch targets whose handover drains what it supersedes rather than cutting it. */
  #drainsSuperseded: WeakSet<SubscribeRequest> = new WeakSet()

  /** Set once by `MOQtailClient.switch`; invoked with everything older on cutover. */
  public onSuperseded?: (superseded: SubscribeRequest[]) => void

  /** Set by the client to decide whether draining a request should continue. */
  public onDrainDecision: (request: SubscribeRequest) => boolean = () => true

  /**
   * How long a drained request may stay quiet before it is retired anyway, in
   * milliseconds. The publisher normally ends the drain with a PUBLISH_DONE; this is
   * what keeps a request that never gets one from being tracked forever.
   */
  public drainTimeoutMs: number = DEFAULT_SWITCH_DRAIN_TIMEOUT_MS

  constructor(initial: SubscribeRequest) {
    this.stream = initial.stream
    this.controller = initial.controller
    initial.manager = this
    this.#requests = [initial]
  }

  /** Requests still tracked, oldest first, draining ones last. */
  public get requests(): readonly SubscribeRequest[] {
    return [...this.#requests, ...this.#draining.keys()]
  }

  /** The request most recently switched to; must deliver data to win the handover. */
  public get newest(): SubscribeRequest {
    return this.#requests[this.#requests.length - 1]!
  }

  /**
   * Track a newly issued switch target alongside whatever is still delivering.
   *
   * `drainSuperseded` says what its handover does to the requests it takes over from:
   * a soft switch leaves them delivering to the end of what the publisher was told to
   * finish, a hard one retires them on the spot.
   */
  public addSwitchTarget(request: SubscribeRequest, drainSuperseded = false): void {
    request.manager = this
    if (drainSuperseded) this.#drainsSuperseded.add(request)
    this.#requests.push(request)
  }

  /** Stop tracking `request` (e.g. it naturally completed without ever losing/winning a handover). */
  public drop(request: SubscribeRequest): void {
    this.#clearDrain(request)
    this.#requests = this.#requests.filter((r) => r !== request)
  }

  /** Stop tracking everything (used when the whole switch chain is torn down at once). */
  public clear(): void {
    for (const request of [...this.#draining.keys()]) this.#clearDrain(request)
    this.#requests = []
  }

  /** Starts, or on each further object restarts, the quiet period that ends a drain. */
  #keepDraining(request: SubscribeRequest): void {
    this.#clearDrain(request)
    this.#draining.set(
      request,
      setTimeout(() => {
        this.#draining.delete(request)
        logger.debug('Subscription', `drain of request ${request.requestId} ended on silence`)
        this.onSuperseded?.([request])
      }, this.drainTimeoutMs),
    )
  }

  #clearDrain(request: SubscribeRequest): void {
    const timeout = this.#draining.get(request)
    if (timeout === undefined) return
    clearTimeout(timeout)
    this.#draining.delete(request)
  }

  /**
   * Delivers an object received for `request`. The first object delivered by the
   * newest tracked request retires everything older — only then is it guaranteed no
   * more data is coming from them.
   *
   * Returns `true` if the object was delivered to the caller,
   * `false` if it was not consumed, and client should cancel the stream.
   */
  public deliver(request: SubscribeRequest, obj: MoqtObject): boolean {
    if (this.#draining.has(request)) {
      // The tail of a track handed over by a soft switch: it covers the media up to
      // where the new one begins, so dropping it is the hole the drain exists to avoid.
      if (this.onDrainDecision?.(request)) {
        this.#keepDraining(request)
        this.controller.enqueue(obj)
        return true
      }
      return false
    }

    if (!this.#requests.includes(request)) {
      logger.warn('Subscription', `discarding object from superseded request ${request.requestId}`)
      return false
    }

    if (request === this.newest && this.#requests.length > 1) {
      const superseded = this.#requests.slice(0, -1)
      this.#requests = [request]
      if (this.#drainsSuperseded.has(request)) {
        for (const old of superseded) this.#keepDraining(old)
      } else {
        this.onSuperseded?.(superseded)
      }
    }

    this.controller.enqueue(obj)
    return true
  }
}
