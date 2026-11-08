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

import type { EndOfRangeHandler } from '@/client/types'
import {
  FullTrackName,
  Location,
  MessageParameter,
  MoqtObject,
  Subscribe,
  RequestError,
  SubscribeOk,
  RequestUpdate,
  PublishDone,
  applyMessageParameterUpdate,
} from '@/model'
import { DEFAULT_SWITCH_DRAIN_TIMEOUT_MS, type EarlyDiscardPolicyConfig } from '@/client/types'
import { logger } from '@/util/logger'
import { isSessionClosed } from '@/client/util/stream_reset'
import type { ObjectOrderer } from '@/client/ordering'
import { ObjectStream } from '@/client/util/object_stream'

// TODO: Add timeout mechanism for unsubscribing

/**
 * Represents an active subscription request in MOQT, implementing `PromiseLike`
 * for convenient async resolution when subscription setup succeeds or fails.
 */
export class SubscribeRequest implements PromiseLike<SubscribeOk | RequestError> {
  public requestId: bigint
  public fullTrackName: FullTrackName
  public isCanceled: boolean = false
  public startLocation: Location | undefined
  public endGroup: bigint | undefined
  public priority: number
  public forward: boolean
  public subscribeParameters: MessageParameter[]
  public earlyDiscardPolicy: EarlyDiscardPolicyConfig | undefined
  public onEndOfRange: EndOfRangeHandler | undefined

  public largestLocation: Location | undefined // Updated on each received object
  /**
   * The publisher's Largest Object as of the last SUBSCRIBE_OK or REQUEST_UPDATE_OK,
   * which is where a fill fetch stream stops.
   */
  public fillBoundary: Location | undefined
  /** A fill fetch stream opened for this subscription. */
  public fillOpened = false
  /** Fill fetch streams still delivering. */
  public openFills = 0
  /** EXPIRES from the last SUBSCRIBE_OK or REQUEST_UPDATE_OK, in milliseconds. */
  public expires: bigint | undefined
  public streamsAccepted: bigint = 0n
  public expectedStreams: bigint | undefined // Defined upon SUBSCRIBE_DONE
  /** The last PUBLISH_DONE received for this request. */
  public publishDone: PublishDone | undefined
  readonly #finished = Promise.withResolvers<PublishDone | undefined>()
  /** Resolves once this request is over, with {@link SubscribeRequest.publishDone}. Never rejects. */
  public readonly done = this.#finished.promise

  /** The Subscription that owns this request's consumer stream; it sets itself on construction. */
  public manager!: Subscription

  public readonly objects = new ObjectStream()
  public readonly stream = this.objects.readable
  public orderer: ObjectOrderer | undefined

  #promise: Promise<SubscribeOk | RequestError>
  #resolve!: (value: SubscribeOk | RequestError | PromiseLike<SubscribeOk | RequestError>) => void
  #reject!: (reason?: any) => void

  constructor(msg: Subscribe) {
    this.requestId = msg.requestId
    this.fullTrackName = msg.fullTrackName

    const filter = msg.parameters.find(MessageParameter.isLocationFilter)
    this.startLocation = filter?.absoluteStart
    this.endGroup = filter?.endLocation()?.group

    const subPriority = msg.parameters.find(MessageParameter.isSubscriberPriority)
    this.priority = subPriority?.priority ?? 128

    const fwd = msg.parameters.find(MessageParameter.isForward)
    this.forward = fwd?.forward ?? true

    this.subscribeParameters = msg.parameters

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
    // An update changes the filter only when it carries one.
    const filter = msg.parameters.find(MessageParameter.isLocationFilter)
    if (filter) {
      this.startLocation = filter.absoluteStart
      this.endGroup = filter.endLocation()?.group
    }

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
   * Puts an object on the consumer stream, through {@link SubscribeRequest.orderer} when
   * there is one. `previous` is the object before it on the same fill fetch stream.
   */
  public enqueue(obj: MoqtObject, previous?: Location): void {
    if (this.orderer) this.orderer.push(obj, previous)
    else this.output(obj)
  }

  /** Writes an object the orderer released, or an unordered one, through the manager. */
  public output(obj: MoqtObject): void {
    this.manager.output(this, obj)
  }

  /** Ends the request: resolves {@link SubscribeRequest.done}. Later calls do nothing. */
  public finish(): void {
    this.#finished.resolve(this.publishDone)
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
    if (!isSessionClosed(reason)) logger.error('request/subscribe', `rejected requestId=${this.requestId}`, reason)
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
  public readonly objects: ObjectStream
  public readonly stream: ReadableStream<MoqtObject>

  /** Oldest first; the last entry is the newest switch target still racing to produce data. */
  #requests: SubscribeRequest[]

  /** Requests {@link Subscription.done} has yet to wait for, oldest first. */
  #unwaited: SubscribeRequest[]

  /**
   * Resolves once every tracked request is over, with the newest one's PUBLISH_DONE, or
   * `undefined` if it ended without one. Never rejects.
   */
  public readonly done: Promise<PublishDone | undefined>

  /**
   * Requests a soft switch handed over from, still delivering the tail of what they
   * were told to finish, each with the timer that retires it if it goes quiet.
   */
  #draining: Map<SubscribeRequest, ReturnType<typeof setTimeout>> = new Map()

  /** Switch targets whose handover drains what it supersedes rather than cutting it. */
  #drainsSuperseded: WeakSet<SubscribeRequest> = new WeakSet()

  /** The newest request's ordered objects, held while older requests still deliver. */
  #parked: MoqtObject[] = []

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
    this.objects = initial.objects
    initial.manager = this
    this.#requests = [initial]
    this.#unwaited = [initial]
    this.done = this.#allDone()
  }

  async #allDone(): Promise<PublishDone | undefined> {
    let batch: SubscribeRequest[] = []
    // A switch may add a request while earlier ones are waited for; batches keep finished ones from piling up
    while (this.#unwaited.length > 0) {
      batch = this.#unwaited.splice(0)
      await Promise.all(batch.map((request) => request.done))
    }
    return batch.at(-1)?.publishDone
  }

  /** Requests still tracked, oldest first, draining ones last. */
  public get requests(): readonly SubscribeRequest[] {
    return [...this.#requests, ...this.#draining.keys()]
  }

  /** The request most recently switched to; must deliver data to win the handover. */
  public get newest(): SubscribeRequest {
    return this.#requests[this.#requests.length - 1]!
  }

  /** More than one request tracked, draining ones included. */
  get #switching(): boolean {
    return this.#requests.length + this.#draining.size > 1
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
    this.#unwaited.push(request)
    this.#updateSwitchState()
  }

  /**
   * Writes an object `request` released. While older requests still deliver, the newest
   * one's ordered objects wait, so the old track's tail comes first.
   */
  public output(request: SubscribeRequest, obj: MoqtObject): void {
    if (request.orderer && request === this.newest && this.#switching) this.#parked.push(obj)
    else this.objects.enqueue(obj)
  }

  /**
   * While a switch is in progress no ordering window expires, and once it's over the
   * parked objects go out first. `left` stopped being tracked.
   */
  #updateSwitchState(left: readonly SubscribeRequest[] = []): void {
    const switching = this.#switching
    // A dropped target can leave one request draining: its tail still goes first
    if (!switching && this.#draining.size === 0) for (const obj of this.#parked.splice(0)) this.objects.enqueue(obj)
    for (const request of this.requests) {
      if (switching) request.orderer?.suspendWindow('switch')
      else request.orderer?.resumeWindow('switch')
    }
    for (const request of left) request.orderer?.resumeWindow('switch')
  }

  /** Hands `requests` to `onSuperseded`, which flushes what they still hold, then stops tracking them. */
  #retire(requests: SubscribeRequest[]): void {
    this.onSuperseded?.(requests)
    this.#updateSwitchState(requests)
  }

  /** Stop tracking `request` (e.g. it naturally completed without ever losing/winning a handover). */
  public drop(request: SubscribeRequest): void {
    this.#clearDrain(request)
    this.#requests = this.#requests.filter((r) => r !== request)
    this.#updateSwitchState([request])
  }

  /** Stop tracking everything (used when the whole switch chain is torn down at once). */
  public clear(): void {
    const left = this.requests
    for (const request of [...this.#draining.keys()]) this.#clearDrain(request)
    this.#requests = []
    this.#parked = []
    this.#updateSwitchState(left)
  }

  /** Starts, or on each further object restarts, the quiet period that ends a drain. */
  #keepDraining(request: SubscribeRequest): void {
    this.#clearDrain(request)
    this.#draining.set(
      request,
      setTimeout(() => {
        this.#draining.delete(request)
        logger.debug('Subscription', `drain of request ${request.requestId} ended on silence`)
        // Retiring flushes what it still holds, ahead of the parked objects
        this.#retire([request])
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
  public deliver(request: SubscribeRequest, obj: MoqtObject, previous?: Location): boolean {
    if (this.#draining.has(request)) {
      // The tail of a track handed over by a soft switch: it covers the media up to
      // where the new one begins, so dropping it is the hole the drain exists to avoid.
      if (this.onDrainDecision(request)) {
        this.#keepDraining(request)
        request.enqueue(obj, previous)
        return true
      }
      // The consumer has what it needs from this track: the drain is over, and what the
      // new track has waiting goes out now rather than after a PUBLISH_DONE or the timeout.
      this.#clearDrain(request)
      this.#retire([request])
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
        this.#retire(superseded)
      }
    }

    request.enqueue(obj, previous)
    return true
  }
}

if (import.meta.vitest) {
  const { test, expect, vi } = import.meta.vitest
  const { ObjectOrderer } = await import('@/client/ordering')
  const { ObjectForwardingPreference } = await import('@/model/data/constant')

  const ftn = FullTrackName.tryNew('room', 'video')
  function orderedRequest(id: bigint): SubscribeRequest {
    const request = new SubscribeRequest(new Subscribe(id, ftn, []))
    request.orderer = new ObjectOrderer(
      { window: { durationMs: 150 } },
      (obj) => request.output(obj),
      () => {},
    )
    return request
  }
  const object = (group: bigint, id: bigint) =>
    MoqtObject.newWithPayload(
      ftn,
      new Location(group, id),
      0,
      ObjectForwardingPreference.Subgroup,
      0,
      null,
      new Uint8Array([1]),
    )

  test('a switch target that ends mid-drain still waits behind the old track’s tail', async () => {
    const old = orderedRequest(0n)
    const subscription = new Subscription(old)
    subscription.drainTimeoutMs = 10
    const target = orderedRequest(1n)
    subscription.addSwitchTarget(target, true)
    const out: string[] = []
    void (async () => {
      for await (const obj of subscription.stream) out.push(`${obj.location.group}.${obj.location.object}`)
    })()

    subscription.deliver(old, object(1n, 0n))
    subscription.deliver(target, object(2n, 0n)) // the handover: the old track drains
    subscription.drop(target)
    subscription.deliver(old, object(1n, 1n))
    await vi.waitFor(() => expect(out).toEqual(['1.0', '1.1', '2.0']))
  })
}
