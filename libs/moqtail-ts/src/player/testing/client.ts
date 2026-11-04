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

/** A relay-free stand-in for `MOQtailClient`. Never exported from the player entry. */

import type { MOQtailClient } from '../../client/client'
import type {
  ClientEvents,
  FetchOptions,
  FetchResult,
  MOQtailClientOptions,
  SubscribeOptions,
  SubscribeResult,
  SubscribeUpdateOptions,
  SwitchOptions,
} from '../../client/types'
import { Location } from '../../model/common/location'
import { ReasonPhrase } from '../../model/common/reason_phrase'
import { PublishDoneStatusCode, RequestErrorCode } from '../../model/control/constant'
import { FetchOk } from '../../model/control/fetch_ok'
import { PublishDone } from '../../model/control/publish_done'
import { RequestError } from '../../model/control/request_error'
import { Setup } from '../../model/control/setup'
import { SubscribeOk } from '../../model/control/subscribe_ok'
import { ObjectForwardingPreference } from '../../model/data/constant'
import { FullTrackName } from '../../model/data/full_track_name'
import { MoqtObject } from '../../model/data/object'
import type { KeyValuePair } from '../../model/common/pair'
import type { TrackProperty } from '../../model/property/track_property'
import { Emitter } from '../../util/emitter'
import { factory } from '../client_slot'

/** How the fake answers a SUBSCRIBE or FETCH: refuse it, or accept it with these values. */
export type FakeResponse = RequestError | { trackProperties?: TrackProperty[]; endOfTrack?: boolean }

/** One accepted SUBSCRIBE or FETCH. The test pushes objects into it and ends it. */
export class FakeRequest {
  readonly stream: ReadableStream<MoqtObject>
  readonly #controller: ReadableStreamDefaultController<MoqtObject>
  readonly #done = Promise.withResolvers<PublishDone | undefined>()
  #open = true

  /** `into` is the subscription a switch target delivers to. */
  constructor(
    readonly kind: 'subscribe' | 'fetch',
    readonly requestId: bigint,
    readonly options: SubscribeOptions | FetchOptions,
    into?: FakeRequest,
  ) {
    let controller!: ReadableStreamDefaultController<MoqtObject>
    // highWaterMark 0: nothing is pulled ahead, so `unread` shows the reader's backpressure.
    this.stream = into?.stream ?? new ReadableStream({ start: (c) => void (controller = c) }, { highWaterMark: 0 })
    this.#controller = into ? into.#controller : controller
  }

  get done(): Promise<PublishDone | undefined> {
    return this.#done.promise
  }

  /** Objects pushed and not read yet. */
  get unread(): number {
    return Math.max(0, -(this.#controller.desiredSize ?? 0))
  }

  push(...objects: MoqtObject[]): void {
    for (const object of objects) this.#controller.enqueue(object)
  }

  /** Closes the stream; a subscription then ends with a PUBLISH_DONE of `status`, or without one. */
  end(status?: PublishDoneStatusCode): void {
    if (!this.#open) return
    this.#open = false
    this.#controller.close()
    this.#done.resolve(status === undefined ? undefined : new PublishDone(status, 0n, new ReasonPhrase('')))
  }

  /** Errors the stream, as a reset or a lost session does. */
  reset(reason: unknown = new Error('reset')): void {
    if (!this.#open) return
    this.#open = false
    this.#controller.error(reason)
    this.#done.resolve(undefined)
  }
}

/** Stands in for the client's session lifecycle and the requests media sources send. */
export class FakeClient extends Emitter<ClientEvents> {
  disconnected = false
  /** Every request method call, in order. */
  readonly calls: { method: string; args: unknown }[] = []
  /** Accepted requests, in order. */
  readonly requests: FakeRequest[] = []
  /** Decides each SUBSCRIBE and FETCH, accepting by default. A pending promise holds the response. */
  respond: (
    kind: 'subscribe' | 'fetch',
    options: SubscribeOptions | FetchOptions,
  ) => FakeResponse | Promise<FakeResponse> = () => ({})
  /** The peer's SETUP. */
  serverSetup = new Setup([])
  /** Decides each REQUEST_UPDATE; throw to fail it. */
  update: (args: SubscribeUpdateOptions) => void | Promise<void> = () => {}
  #terminated = false
  #nextId = 0n

  constructor(
    readonly options: MOQtailClientOptions = { url: 'https://relay.test' },
    readonly opened: Promise<void> = Promise.resolve(),
  ) {
    super()
  }

  init(): Promise<void> {
    return this.opened
  }

  async disconnect(reason?: unknown): Promise<void> {
    this.disconnected = true
    this.drop(reason)
  }

  /** Ends the session, as a lost connection does: open requests error. */
  drop(reason?: unknown): void {
    if (this.#terminated) return
    this.#terminated = true
    for (const request of this.requests) request.reset(reason)
    this.emit('sessionterminated', reason)
  }

  /** The arguments of every call to `method`. */
  sent<T = unknown>(method: string): T[] {
    return this.calls.filter((c) => c.method === method).map((c) => c.args as T)
  }

  async subscribe(options: SubscribeOptions): Promise<RequestError | SubscribeResult> {
    const response = await this.#respond('subscribe', options)
    if (response instanceof RequestError) return response
    const request = this.#open('subscribe', options)
    const subscribeOk = SubscribeOk.create(request.requestId, [], response.trackProperties ?? [])
    return { requestId: request.requestId, subscribeOk, stream: request.stream, done: request.done }
  }

  async fetch(options: FetchOptions): Promise<RequestError | FetchResult> {
    const response = await this.#respond('fetch', options)
    if (response instanceof RequestError) return response
    const request = this.#open('fetch', options)
    const fetchOk = new FetchOk(response.endOfTrack ?? false, new Location(0n, 0n), [], response.trackProperties ?? [])
    return { requestId: request.requestId, fetchOk, stream: request.stream }
  }

  /** The new request delivers into the subscription it switches from. */
  async switch(args: SwitchOptions): Promise<RequestError | SubscribeResult> {
    const from = this.request(args.switchFromRequestId)
    const response = await this.#respond('switch', args, args.newSubscribeOptions)
    if (response instanceof RequestError) return response
    const request = this.#open('subscribe', args.newSubscribeOptions, from)
    const subscribeOk = SubscribeOk.create(request.requestId, [], response.trackProperties ?? [])
    return { requestId: request.requestId, subscribeOk, stream: request.stream, done: from?.done ?? request.done }
  }

  async subscribeUpdate(args: SubscribeUpdateOptions): Promise<void> {
    this.#record('subscribeUpdate', args)
    await this.update(args)
  }

  async unsubscribe(requestId: bigint): Promise<void> {
    this.#record('unsubscribe', requestId)
    this.request(requestId)?.end()
  }

  async fetchCancel(requestId: bigint): Promise<void> {
    this.#record('fetchCancel', requestId)
    this.request(requestId)?.end()
  }

  request(requestId: bigint): FakeRequest | undefined {
    return this.requests.find((r) => r.requestId === requestId)
  }

  /** The latest accepted request. */
  get last(): FakeRequest {
    const request = this.requests.at(-1)
    if (!request) throw new Error('no request yet')
    return request
  }

  #record(method: string, args: unknown): void {
    // As the real client does once the session is gone.
    if (this.#terminated) throw new Error(`${method} after the session ended`)
    this.calls.push({ method, args })
  }

  #respond(method: 'subscribe' | 'fetch' | 'switch', args: unknown, options = args): Promise<FakeResponse> {
    this.#record(method, args)
    return Promise.resolve(this.respond(method === 'fetch' ? 'fetch' : 'subscribe', options as SubscribeOptions))
  }

  #open(kind: 'subscribe' | 'fetch', options: SubscribeOptions | FetchOptions, into?: FakeRequest): FakeRequest {
    const request = new FakeRequest(kind, this.#nextId++, options, into)
    this.requests.push(request)
    return request
  }
}

/** A REQUEST_ERROR with `code`, for `respond`. */
export const refuse = (code = RequestErrorCode.DoesNotExist) => new RequestError(code, 0n, new ReasonPhrase('refused'))

/** A media object at `group`.`object` of `track`, with a payload and optional object properties. */
export function mediaObject(
  group: number,
  object = 0,
  properties: KeyValuePair[] | null = null,
  track = 'v1',
): MoqtObject {
  const name = FullTrackName.tryNew('ns', track)
  const location = new Location(BigInt(group), BigInt(object))
  return MoqtObject.newWithPayload(
    name,
    location,
    0,
    ObjectForwardingPreference.Subgroup,
    0,
    properties,
    new Uint8Array([group, object]),
  )
}

/**
 * Makes the client slot create `FakeClient`s. `open` decides how each `init()` settles,
 * by creation index. Returns the clients created so far and a restore function.
 */
export function fakeClients(open: (index: number) => Promise<void> = () => Promise.resolve()) {
  const clients: FakeClient[] = []
  const saved = factory.createClient
  factory.createClient = (options) => {
    const client = new FakeClient(options, open(clients.length))
    clients.push(client)
    return client as unknown as MOQtailClient
  }
  return { clients, restore: () => (factory.createClient = saved) }
}
