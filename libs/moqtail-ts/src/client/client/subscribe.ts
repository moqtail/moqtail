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

import { ObjectOrderer } from '@/client/ordering'
import { SubscribeRequest, Subscription } from '@/client/request/subscribe'
import { assertWithinFilterRangeLimit, isValidTrackAlias } from '@/client/util/validators'
import type {
  ObjectOrdering,
  SubscribeOptions,
  SubscribeResult,
  SubscribeUpdateOptions,
  SwitchOptions,
} from '@/client/types'
import { GroupOrder, SwitchMode } from '@/model/constants'
import type { MoqtObject } from '@/model/data/object'
import { InternalError, ProtocolViolationError } from '@/model/error/error'
import { StreamResetCode } from '@/model/error/stream_reset'
import { isRequestError } from '@/model/messages'
import type { RequestError, RequestUpdate, Subscribe } from '@/model/messages'
import { applyParameterUpdate } from '@/model/parameters'
import type { LocationFilter, MessageParameters } from '@/model/parameters'
import { errorMessage } from '@/util/error'
import { logger } from '@/util/logger'
import { claimTrackAlias, releaseTrackAlias } from './aliases'
import { closeRequestStream, openRequestStream, requestStreamFor, resetRequestStream } from './request_streams'
import { closeSession } from './session'
import type { Session } from './session'

/** The options a SUBSCRIBE or FETCH spells out, besides its other parameters. */
interface RequestOptions {
  priority?: number | undefined
  forward?: boolean | undefined
  groupOrder?: GroupOrder | undefined
  locationFilter?: LocationFilter | undefined
  parameters?: MessageParameters | undefined
}

export async function subscribe(session: Session, args: SubscribeOptions): Promise<RequestError | SubscribeResult> {
  const { client } = session
  session.ensureActive()
  // Checked before the try: a request the caller got wrong is not a broken session.
  assertWithinFilterRangeLimit(args.parameters ?? {}, client.peerMaxFilterRanges, 'MOQtailClient.subscribe')

  try {
    const { fullTrackName, groupOrder } = args
    const msg: Subscribe = {
      kind: 'subscribe',
      requestId: session.nextRequestId(),
      fullTrackName,
      parameters: requestParameters(args),
    }
    const request = new SubscribeRequest(msg)
    const subscription = new Subscription(request)

    request.earlyDiscardPolicy = args.earlyDiscardPolicy
    request.onEndOfRange = args.onEndOfRange
    request.orderer = createOrderer(session, args.ordering, groupOrder, request.requestId, (obj) => request.output(obj))

    // The fill brings what comes before the live objects: until it ends, none of it is missing
    const fills = args.parameters?.fillParameters !== undefined
    if (fills) request.orderer?.suspendWindow('fill')

    client.requests.set(request.requestId, request)
    client.requestIdMap.addMapping(request.requestId, request.fullTrackName)

    await openRequestStream(session, request.requestId, msg)

    const response = await request

    if (isRequestError(response)) {
      client.requests.delete(request.requestId)
      client.requestIdMap.removeMappingByRequestId(request.requestId)
      return response
    }

    claimTrackAlias(session, response.trackAlias, request)

    if (fills) liftFillSuspension(session, request)

    return { requestId: msg.requestId, subscribeOk: response, stream: request.stream, done: subscription.done }
  } catch (error) {
    const reason = errorMessage(error)
    if (!session.closing(error)) logger.error('MOQtailClient', `subscribe: unexpected error — ${reason}`)

    await closeSession(session, new InternalError('MOQtailClient.subscribe', reason))
    throw error
  }
}

export async function unsubscribe(session: Session, requestId: bigint): Promise<void> {
  session.ensureActive()

  const subscription = session.client.requests.get(requestId)
  if (!(subscription instanceof SubscribeRequest)) return

  // A switched subscription may still be tracking several requests racing to take over
  // (soft-switch handover); unsubscribing tears down the whole chain, not just the id
  // passed in.
  const owner = subscription.manager
  const targets = owner ? owner.requests.slice() : [subscription]

  try {
    for (const target of targets) {
      // There is no UNSUBSCRIBE. Resetting the subscription's request stream is what
      // tells the publisher to stop, and the code it reads back off that reset is CANCELLED.
      await resetRequestStream(session, target.requestId, StreamResetCode.Cancelled)
      target.unsubscribe()
    }
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.unsubscribe', errorMessage(error)))
    throw error
  } finally {
    for (const target of targets) forgetSubscription(session, target)
    owner?.clear()
    ;(owner ?? subscription).objects.close()
  }
}

export async function subscribeUpdate(session: Session, args: SubscribeUpdateOptions): Promise<void> {
  const { client } = session
  session.ensureActive()
  const { subscriptionRequestId, priority, forward, parameters, locationFilter } = args
  // The limit holds for the update itself and for the filters the subscription is left
  // with once it applies.
  const limit = client.peerMaxFilterRanges
  assertWithinFilterRangeLimit(parameters ?? {}, limit, 'MOQtailClient.subscribeUpdate')

  const request = client.requests.get(subscriptionRequestId)
  if (!(request instanceof SubscribeRequest)) return

  const resulting = applyParameterUpdate(request.subscribeParameters, parameters ?? {})
  assertWithinFilterRangeLimit(resulting, limit, 'MOQtailClient.subscribeUpdate')

  try {
    const trackAlias = client.subscriptionAliasMap.get(subscriptionRequestId)
    if (!isValidTrackAlias(trackAlias)) {
      throw new InternalError('MOQtailClient.subscribeUpdate', 'Request exists but track alias mapping does not')
    }

    const subscription = client.subscriptions.get(trackAlias)
    if (!subscription) {
      throw new InternalError('MOQtailClient.subscribeUpdate', 'Request exists but subscription does not')
    }

    const msg: RequestUpdate = {
      kind: 'requestUpdate',
      requestId: session.nextRequestId(),
      parameters: requestParameters({ priority, forward, locationFilter, parameters }),
    }

    // The request and the subscription are the same object in both maps
    subscription.update(msg)

    // A REQUEST_UPDATE travels on the stream of the request it updates.
    await requestStreamFor(session, subscriptionRequestId, 'MOQtailClient.subscribeUpdate').send(msg)
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.subscribeUpdate', errorMessage(error)))
    throw error
  }
}

export async function switchTrack(session: Session, args: SwitchOptions): Promise<RequestError | SubscribeResult> {
  const { client } = session
  session.ensureActive()
  const { switchFromRequestId, switchMode, newSubscribeOptions, onDrainDecision } = args
  assertWithinFilterRangeLimit(newSubscribeOptions.parameters ?? {}, client.peerMaxFilterRanges, 'MOQtailClient.switch')

  try {
    const fromRequest = client.requests.get(switchFromRequestId)
    if (!(fromRequest instanceof SubscribeRequest) || !fromRequest.manager) {
      throw new ProtocolViolationError('MOQtailClient.switch', 'switchFromRequestId is not an active subscription')
    }

    const subscription = fromRequest.manager

    subscription.onSuperseded ??= (superseded) => {
      for (const request of superseded) void retireSwitchedRequest(session, request)
    }

    subscription.drainTimeoutMs = client.switchDrainTimeoutMs
    if (onDrainDecision) subscription.onDrainDecision = onDrainDecision

    // A soft switch leaves the track it switches away from delivering up to the group this
    // one starts at, so the two meet without a hole. Asking for its PUBLISH_DONE is what
    // says the drain is over: without one nothing on the wire tells a track still
    // finishing from one that has stopped.
    const drains = switchMode === SwitchMode.Soft
    const switchFrom = { requestId: switchFromRequestId, mode: switchMode, publishDone: drains }
    const parameters = { ...newSubscribeOptions.parameters, switchFrom }

    // subscribe() gives the new request its own stream, but nothing reads it:
    // Subscription.deliver() re-routes its objects onto the original stream.
    const result = await subscribe(session, { ...newSubscribeOptions, parameters })
    if (isRequestError(result)) return result

    const newRequest = client.requests.get(result.requestId) as SubscribeRequest
    subscription.addSwitchTarget(newRequest, drains)

    return { ...result, stream: subscription.stream, done: subscription.done }
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.switch', errorMessage(error)))
    throw error
  }
}

/**
 * The orderer for a request, unless its ordering, else the client's, is `'none'` or unset.
 * Descending group order is never ordered: every group after the first would be late.
 */
export function createOrderer(
  session: Session,
  ordering: ObjectOrdering | 'none' | undefined,
  groupOrder: GroupOrder,
  requestId: bigint,
  release: (obj: MoqtObject) => void,
): ObjectOrderer | undefined {
  const resolved = ordering ?? session.options.ordering ?? 'none'
  if (resolved === 'none' || groupOrder === GroupOrder.Descending) return undefined

  return new ObjectOrderer(resolved, release, (from, to) => session.client.emit('orderinggap', requestId, from, to))
}

/**
 * Ends a subscription whose publisher has said how many streams it opened and has now
 * delivered all of them, and reports whether that happened.
 *
 * A PUBLISH_DONE carries the count, and the streams finish whenever they finish. Either
 * can be last, so this is checked from both: a drained switch target has usually finished
 * its streams well before the PUBLISH_DONE that ends it.
 */
export function completeSubscription(session: Session, request: SubscribeRequest, force: boolean): boolean {
  if (!force && (!request.expectedStreams || request.streamsAccepted < request.expectedStreams)) return false

  request.orderer?.flush()

  const owner = request.manager
  if (owner) {
    owner.drop(request)

    if (owner.requests.length === 0) owner.objects.close()
  } else {
    request.objects.close()
  }

  forgetSubscription(session, request)

  // The publisher is done with it, so nothing is being cancelled: close the stream it ran
  // on rather than resetting it.
  void closeRequestStream(session, request.requestId)

  return true
}

/**
 * The parameters of a request: its other parameters, with the options it spells out
 * over them. GROUP_ORDER travels only when it is not ORIGINAL (moqt-22 §9.2.2.6).
 */
export function requestParameters({
  priority,
  forward,
  groupOrder,
  locationFilter,
  parameters,
}: RequestOptions): MessageParameters {
  const params: MessageParameters = { ...parameters }

  if (priority !== undefined) params.subscriberPriority = priority
  if (forward !== undefined) params.forward = forward
  if (groupOrder !== undefined && groupOrder !== GroupOrder.Original) params.groupOrder = groupOrder
  if (locationFilter) params.locationFilter = locationFilter

  return params
}

/**
 * Lifts the fill's ordering suspension when no fill fetch stream will open: nothing was
 * published yet (no LARGEST_OBJECT), or none opened shortly after SUBSCRIBE_OK (an empty
 * fill range, moqt-22 3.4).
 */
function liftFillSuspension(session: Session, request: SubscribeRequest): void {
  const lift = () => {
    if (!request.fillOpened) request.orderer?.resumeWindow('fill')
  }

  if (!request.fillBoundary) lift()
  else setTimeout(lift, session.client.fillOpenTimeoutMs)
}

/** Tears down a switch target that lost the handover race to a newer one. */
async function retireSwitchedRequest(session: Session, request: SubscribeRequest): Promise<void> {
  // Before the track it hands over to delivers: nothing of this one may follow that
  request.orderer?.flush()
  forgetSubscription(session, request)

  await resetRequestStream(session, request.requestId, StreamResetCode.Cancelled)
}

/** Drops `request` from the request tables, frees its alias, and resolves its `done`. */
function forgetSubscription(session: Session, request: SubscribeRequest): void {
  session.client.requests.delete(request.requestId)
  session.client.requestIdMap.removeMappingByRequestId(request.requestId)
  releaseTrackAlias(session, request)
  request.finish()
}

if (import.meta.vitest) {
  const { describe, it, expect, vi } = import.meta.vitest
  const { MOQtailClient } = await import('@/client/client/client')
  const { MockWebTransport, connected, openedStream, restoreWebTransport, subscribeOk } =
    await import('@/client/testing')
  const { LiveTrackSource } = await import('@/client/track/content_source')
  const { streamResetCodeOf } = await import('@/client/util/stream_reset')
  const { ByteBuffer } = await import('@/model/common/byte_buffer')
  const { Location } = await import('@/model/common/location')
  const { Tuple } = await import('@/model/common/tuple')
  const { LocationFilterType, PublishDoneStatusCode, RequestErrorCode } = await import('@/model/constants')
  const { FetchHeaderType, ObjectForwardingPreference } = await import('@/model/data/constant')
  const { FetchObject } = await import('@/model/data/fetch_object')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { Header } = await import('@/model/data/header')
  const { MoqtObject } = await import('@/model/data/object')
  const { SubgroupObject } = await import('@/model/data/subgroup_object')
  const { InvalidFilterError, encodeRanges, locationFilter } = await import('@/model/parameters')
  type FetchObjectContext = import('@/model/data/fetch_object').FetchObjectContext
  type FullTrackName = import('@/model/data/full_track_name').FullTrackName
  type MOQtailClient = import('@/client/client/client').MOQtailClient
  type MockWebTransport = import('@/client/testing').MockWebTransport
  type PublishDone = import('@/model/messages').PublishDone
  type SubscribePublication = import('@/client/publication/subscribe').SubscribePublication

  describe('MOQtailClient subscriptions', () => {
    restoreWebTransport()

    const ftn = FullTrackName.tryNew('room/alice', 'video')

    it('keeps the alias of a switch back to a track whose earlier request is still retiring', async () => {
      const { client, transport } = await connected()

      client.switchDrainTimeoutMs = 10

      const other = FullTrackName.tryNew('room/alice', 'video-low')
      const options = {
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      } as const

      const first = client.subscribe({ ...options, fullTrackName: ftn })

      ;(await openedStream(transport, 0)).respond(subscribeOk(7n, {}, []))

      const firstId = ((await first) as { requestId: bigint }).requestId

      // Away to another track, then straight back: the relay names a track, so the
      // return trip is handed the same alias while the first request still holds it.
      const away = client.switch({
        switchFromRequestId: firstId,
        switchMode: SwitchMode.Soft,
        newSubscribeOptions: { ...options, fullTrackName: other },
      })

      ;(await openedStream(transport, 1)).respond(subscribeOk(8n, {}, []))

      const switched = (await away) as SubscribeResult
      const awayId = switched.requestId

      // The new request's SUBSCRIBE_OK, on the original subscription's stream
      expect(switched.subscribeOk.trackAlias).toBe(8n)
      expect(switched.stream).toBe((client.requests.get(firstId) as SubscribeRequest).manager!.stream)

      const back = client.switch({
        switchFromRequestId: awayId,
        switchMode: SwitchMode.Soft,
        newSubscribeOptions: { ...options, fullTrackName: ftn },
      })

      ;(await openedStream(transport, 2)).respond(subscribeOk(7n, {}, []))

      const backId = ((await back) as { requestId: bigint }).requestId

      // Delivering hands over from both older requests, and the first of them was
      // subscribed to this same track, so it holds alias 7 too.
      const newest = client.requests.get(backId) as SubscribeRequest
      newest.manager!.deliver(newest, {} as MoqtObject)

      expect(client.subscriptions.get(7n)).toBe(newest)

      // They drain rather than being cut, and retiring them at the end of that must
      // not take the alias the newest request is using with them.
      await vi.waitFor(() => expect(client.subscriptionAliasMap.get(firstId)).toBeUndefined())

      expect(client.subscriptions.get(7n)).toBe(newest)
      expect(client.aliasFullTrackNameMap.get(7n)?.toString()).toBe(ftn.toString())
      expect(client.subscriptions.has(8n)).toBe(false)

      await client.disconnect()
    })

    it('sends REQUEST_UPDATE on the subscription stream and cancels by closing it', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })

      const subscribeStream = await openedStream(transport, 0)
      const requestId = (subscribeStream.messages[0] as Subscribe).requestId

      subscribeStream.respond(subscribeOk(7n, {}, []))
      await subscribing

      await client.subscribeUpdate({
        subscriptionRequestId: requestId,
        locationFilter: locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 1n, 0n, 5n),
        forward: false,
        priority: 200,
      })

      expect(subscribeStream.messages[1]).toMatchObject({ kind: 'requestUpdate' })
      expect(transport.biStreams).toHaveLength(1)

      // Without priority and forward, neither is sent: only what the update names changes
      await client.subscribeUpdate({ subscriptionRequestId: requestId, parameters: { forward: true } })

      const update = subscribeStream.messages[2] as RequestUpdate
      expect(update.parameters).toEqual({ forward: true })

      await client.subscribeUpdate({ subscriptionRequestId: requestId })
      expect((subscribeStream.messages[3] as RequestUpdate).parameters).toEqual({})

      await client.unsubscribe(requestId)

      // No UNSUBSCRIBE message: closing the stream is the cancellation.
      expect(subscribeStream.messages).toHaveLength(4)
      expect(subscribeStream.isClosed).toBe(true)

      await client.disconnect()
    })

    describe('stops at the end of the Location Filter without ending the subscription', () => {
      async function serve(
        filter: LocationFilter,
        groups: number,
        objectsPerGroup: number,
        rangeFilters: MessageParameters = {},
      ) {
        const { client, transport } = await connected()
        let source!: ReadableStreamDefaultController<MoqtObject>

        client.addOrUpdateTrack({
          fullTrackName: ftn,
          trackSource: {
            live: new LiveTrackSource(new ReadableStream<MoqtObject>({ start: (c) => (source = c) })),
          },
          publisherPriority: 0,
          trackAlias: 7n,
        })
        const incoming = transport.openIncomingBiStream()

        incoming.respond({
          kind: 'subscribe',
          requestId: 4n,
          fullTrackName: ftn,
          parameters: { locationFilter: filter, ...rangeFilters },
        })

        await vi.waitFor(() => expect(incoming.messages).toHaveLength(1))

        const before = transport.sentChunks.length
        const streamsBefore = transport.uniStreamOptions.length

        for (let g = 0n; g < BigInt(groups); g++) {
          for (let o = 0n; o < BigInt(objectsPerGroup); o++) {
            source.enqueue(
              MoqtObject.newWithPayload(
                ftn,
                new Location(g, o),
                0,
                ObjectForwardingPreference.Subgroup,
                0,
                null,
                new Uint8Array([1]),
              ),
            )
          }
        }

        await new Promise((resolve) => setTimeout(resolve, 20))

        return {
          client,
          incoming,
          opened: transport.uniStreamOptions.length - streamsBefore,
          written: transport.sentChunks.length - before,
        }
      }

      it('delivers only the objects its range filters pass', async () => {
        const { client, opened, written } = await serve(
          locationFilter(LocationFilterType.AbsoluteStart, 0n, 0n),
          3,
          4,
          { objectIdFilter: [{ setId: 0, deltas: encodeRanges([{ start: 1n, end: 2n }]) }] },
        )

        // Objects 1 and 2 of each group; every group still gets its stream.
        expect(opened).toBe(3)
        expect(written).toBe(3 + 6)

        await client.disconnect()
      })

      it('ORs its filter sets and ANDs the filters within one', async () => {
        const { client, opened, written } = await serve(
          locationFilter(LocationFilterType.AbsoluteStart, 0n, 0n),
          3,
          2,
          {
            // Set 0 passes nothing: every object has publisher priority 0.
            priorityFilter: [{ setId: 0, deltas: encodeRanges([{ start: 1n }]) }],
            // Set 1 passes object 0 of subgroup 0.
            objectIdFilter: [{ setId: 1, deltas: encodeRanges([{ start: 0n, end: 0n }]) }],
            subgroupFilter: [{ setId: 1, deltas: encodeRanges([{ start: 0n, end: 0n }]) }],
          },
        )

        expect(opened).toBe(3)
        expect(written).toBe(3 + 3)

        await client.disconnect()
      })

      it('opens no stream for objects that are all filtered out', async () => {
        const { client, opened, written } = await serve(
          locationFilter(LocationFilterType.AbsoluteStart, 0n, 0n),
          3,
          2,
          { objectIdFilter: [{ setId: 0, deltas: encodeRanges([{ start: 5n }]) }] },
        )

        expect(opened).toBe(0)
        expect(written).toBe(0)

        await client.disconnect()
      })

      it('applies the filters a REQUEST_UPDATE leaves it with', async () => {
        const { client, incoming } = await serve(locationFilter(LocationFilterType.AbsoluteStart, 0n, 0n), 0, 0)
        const priority = { setId: 0, deltas: encodeRanges([{ start: 1n }]) }

        incoming.respond({ kind: 'requestUpdate', requestId: 4n, parameters: { priorityFilter: [priority] } })

        await vi.waitFor(() => expect(incoming.messages).toHaveLength(2))

        expect(incoming.messages[1]).toMatchObject({ kind: 'requestOk' })

        // A second kind beyond the subscription's own limit is refused, and the update
        // ends it.
        const one = await connected({}, { setupOptions: { maxFilterRanges: 1n } })

        one.client.addOrUpdateTrack({
          fullTrackName: ftn,
          trackSource: { live: new LiveTrackSource(new ReadableStream<MoqtObject>()) },
          publisherPriority: 0,
          trackAlias: 7n,
        })

        const stream = one.transport.openIncomingBiStream()

        stream.respond({
          kind: 'subscribe',
          requestId: 4n,
          fullTrackName: ftn,
          parameters: { priorityFilter: [priority] },
        })

        await vi.waitFor(() => expect(stream.messages).toHaveLength(1))

        const objectId = { setId: 0, deltas: encodeRanges([{ start: 0n, end: 0n }]) }
        stream.respond({ kind: 'requestUpdate', requestId: 4n, parameters: { objectIdFilter: [objectId] } })
        await vi.waitFor(() => expect(stream.messages).toHaveLength(3))

        expect((stream.messages[1] as RequestError).errorCode).toBe(RequestErrorCode.InvalidFilter)
        expect((stream.messages[2] as PublishDone).statusCode).toBe(PublishDoneStatusCode.UpdateFailed)

        await one.client.disconnect()
        await client.disconnect()
      })

      it('delivers the whole of a Group End and nothing after it', async () => {
        const { client, incoming, opened, written } = await serve(
          locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 1n, 0n, 2n),
          4,
          3,
        )

        // Groups 1 and 2: a stream each, and all three objects of both.
        expect(opened).toBe(2)
        expect(written).toBe(2 + 6)

        // No PUBLISH_DONE: the subscription outlives its filter.
        expect(incoming.messages).toHaveLength(1)
        expect((client.publications.get(4n) as SubscribePublication | undefined)?.latestLocation).toEqual(
          new Location(2n, 2n),
        )

        await client.disconnect()
      })

      it('stops after the End Object of an Absolute Range', async () => {
        const { client, incoming, opened, written } = await serve(
          locationFilter(LocationFilterType.AbsoluteRange, 0n, 0n, 1n, 1n),
          3,
          3,
        )

        expect(opened).toBe(2)
        // 0/0..0/2 and 1/0..1/1.
        expect(written).toBe(2 + 5)
        expect(incoming.messages).toHaveLength(1)

        await client.disconnect()
      })

      it('treats an end in group 0 as an end', async () => {
        const { client, opened, written } = await serve(
          locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 0n, 0n, 0n),
          3,
          2,
        )

        expect(opened).toBe(1)
        expect(written).toBe(1 + 2)

        await client.disconnect()
      })
    })

    describe("the server's MAX_FILTER_RANGES", () => {
      const twoRanges = () => ({ setId: 0, deltas: encodeRanges([{ start: 0n, end: 0n }, { start: 4n }]) })

      const subscribeWith = (client: MOQtailClient, parameters: MessageParameters) =>
        client.subscribe({
          fullTrackName: ftn,
          locationFilter: locationFilter(LocationFilterType.NextObject),
          forward: true,
          groupOrder: GroupOrder.Original,
          priority: 0,
          parameters,
        })

      it('defaults to 0 when the server sends none', async () => {
        const { client } = await connected()
        expect(client.peerMaxFilterRanges).toBe(0n)

        await client.disconnect()
      })

      it('keeps a filter the server allows none of from being sent, without ending the session', async () => {
        const { client, transport } = await connected()

        await expect(subscribeWith(client, { objectIdFilter: [twoRanges()] })).rejects.toThrow(InvalidFilterError)
        await expect(
          client.fetch({
            priority: 0,
            groupOrder: GroupOrder.Original,
            fullTrackName: ftn,
            locationFilter: locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 0n, 0n, 1n),
            parameters: { objectIdFilter: [twoRanges()] },
          }),
        ).rejects.toThrow(InvalidFilterError)

        await expect(
          client.subscribeTracks(Tuple.fromUtf8Path('room'), { objectIdFilter: [twoRanges()] }),
        ).rejects.toThrow(InvalidFilterError)

        expect(transport.biStreams).toHaveLength(0)

        // The session carries on: a request without filters still goes out.
        // Left unanswered; the disconnect below rejects it.
        subscribeWith(client, {}).catch(() => {})

        expect((await openedStream(transport, 0)).messages[0]).toMatchObject({ kind: 'subscribe' })

        await client.disconnect()
      })

      it('sends filters within the limit and refuses those beyond it', async () => {
        const { client, transport } = await connected({ maxFilterRanges: 2n })
        expect(client.peerMaxFilterRanges).toBe(2n)

        await expect(
          subscribeWith(client, {
            objectIdFilter: [twoRanges()],
            subgroupFilter: [{ setId: 0, deltas: encodeRanges([{ start: 0n, end: 0n }]) }],
          }),
        ).rejects.toThrow(InvalidFilterError)
        expect(transport.biStreams).toHaveLength(0)

        subscribeWith(client, { objectIdFilter: [twoRanges()] }).catch(() => {})

        const sent = (await openedStream(transport, 0)).messages[0] as Subscribe
        expect(sent.parameters.objectIdFilter).toEqual([twoRanges()])

        await client.disconnect()
      })

      it('holds an update to the filters the subscription is left with', async () => {
        const { client, transport } = await connected({ maxFilterRanges: 2n })
        const subscribing = subscribeWith(client, { objectIdFilter: [twoRanges()] })
        const stream = await openedStream(transport, 0)

        stream.respond(subscribeOk(7n, {}, []))

        const { requestId } = (await subscribing) as { requestId: bigint }

        // Another kind adds to the two ranges already held.
        const subgroup = { setId: 0, deltas: encodeRanges([{ start: 0n, end: 0n }]) }

        await expect(
          client.subscribeUpdate({
            subscriptionRequestId: requestId,
            priority: 0,
            forward: true,
            parameters: { subgroupFilter: [subgroup] },
          }),
        ).rejects.toThrow(InvalidFilterError)

        // Replacing the kind already held does not.
        const narrower = { setId: 0, deltas: encodeRanges([{ start: 1n, end: 1n }]) }
        await client.subscribeUpdate({
          subscriptionRequestId: requestId,
          priority: 0,
          forward: true,
          parameters: { objectIdFilter: [narrower] },
        })

        await client.disconnect()
      })
    })

    it('ends a subscription whose REQUEST_UPDATE fails with PUBLISH_DONE(UPDATE_FAILED)', async () => {
      const { client, transport } = await connected()

      const incoming = transport.openIncomingBiStream()

      // No such track, so the SUBSCRIBE is refused and leaves no subscription to update.
      incoming.respond({
        kind: 'subscribe',
        requestId: 4n,
        fullTrackName: ftn,
        parameters: { locationFilter: locationFilter(LocationFilterType.NextObject) },
      })

      await vi.waitFor(() => expect(incoming.messages).toHaveLength(1))

      expect(incoming.messages[0]).toMatchObject({ kind: 'requestError' })

      incoming.respond({ kind: 'requestUpdate', requestId: 4n, parameters: { subscriberPriority: 200 } })

      await vi.waitFor(() => expect(incoming.messages).toHaveLength(3))
      expect((incoming.messages[1] as RequestError).errorCode).toBe(RequestErrorCode.NotSupported)

      // §10.9.1: a failed update also terminates the subscription.
      const done = incoming.messages[2]
      expect(done).toMatchObject({ kind: 'publishDone' })
      expect((done as PublishDone).statusCode).toBe(PublishDoneStatusCode.UpdateFailed)

      await client.disconnect()
    })

    it('unsubscribes by resetting the SUBSCRIBE stream with CANCELLED', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })

      const subscribeStream = await openedStream(transport, 0)
      subscribeStream.respond(subscribeOk(7n, {}, []))

      const subscribed = await subscribing
      expect(subscribed).not.toMatchObject({ kind: 'requestError' })

      await client.unsubscribe((subscribed as SubscribeResult).requestId)

      expect(await (subscribed as SubscribeResult).done).toBeUndefined()
      expect((await (subscribed as SubscribeResult).stream.getReader().read()).done).toBe(true)

      // §3.3.2: UNSUBSCRIBE is gone. Nothing follows the SUBSCRIBE on the wire — the
      // publisher learns the subscription ended from the reset code.
      expect(subscribeStream.messages).toHaveLength(1)
      expect(streamResetCodeOf(subscribeStream.abortReason)).toBe(StreamResetCode.Cancelled)
      expect(streamResetCodeOf(subscribeStream.cancelReason)).toBe(StreamResetCode.Cancelled)

      await client.disconnect()
    })

    describe('ordering', () => {
      /** Subscribes on alias 7 with `ordering`, with helpers that send its fill and live streams and read it. */
      async function subscribeOrdered(
        client: MOQtailClient,
        transport: MockWebTransport,
        ordering: ObjectOrdering | 'none',
      ) {
        const subscribing = client.subscribe({
          fullTrackName: ftn,
          locationFilter: locationFilter(LocationFilterType.NextObject),
          parameters: { fillParameters: { locationFilter: locationFilter(LocationFilterType.RelativeStart, 2n) } },
          forward: true,
          groupOrder: GroupOrder.Original,
          priority: 0,
          ordering,
        })

        const subscribeStream = await openedStream(transport, 0)
        const requestId = (subscribeStream.messages[0] as Subscribe).requestId

        subscribeStream.respond(subscribeOk(7n, { largestObject: new Location(5n, 1n) }))

        const { stream } = (await subscribing) as { stream: ReadableStream<MoqtObject> }
        const payload = new Uint8Array([1])

        const live = (group: bigint, object: number) => {
          const bytes = new ByteBuffer()
          bytes.putBytes(Header.newSubgroup(0x10, 7n, group, undefined, 0).serialize().toUint8Array())
          bytes.putBytes(SubgroupObject.newWithPayload(object, null, payload).serialize(undefined).toUint8Array())
          transport.openIncomingUniStream(bytes.toUint8Array())
        }

        const fill = (...locations: [number, number][]) => {
          const bytes = new ByteBuffer()
          bytes.putBytes(Header.newFetch(FetchHeaderType.Type0x05, requestId).serialize().toUint8Array())

          let ctx: FetchObjectContext | undefined

          for (const [group, object] of locations) {
            const fetchObject = FetchObject.newObject(
              group,
              0,
              object,
              0,
              ObjectForwardingPreference.Subgroup,
              null,
              payload,
            )

            bytes.putBytes(fetchObject.serialize(ctx, GroupOrder.Ascending).toUint8Array())
            ctx = fetchObject.nextContext(ctx)
          }

          // A fill fetch stream ends once its range is delivered
          transport.openIncomingUniStream(bytes.toUint8Array()).close()
        }

        const read = async (count: number) => {
          const reader = stream.getReader()
          const out: string[] = []

          for (let i = 0; i < count; i++) {
            const { value } = await reader.read()
            out.push(`${value!.location.group}.${value!.location.object}`)
          }

          reader.releaseLock()

          return out
        }

        return { live, fill, read }
      }

      it('orders a fill and one stream per object, and reports what it stopped waiting for', async () => {
        const { client, transport } = await connected()
        const gaps: string[] = []
        client.on('orderinggap', (_, from, to) => gaps.push(`${from.group}.${from.object}-${to.group}.${to.object}`))
        const { live, fill, read } = await subscribeOrdered(client, transport, { window: { durationMs: 100 } })

        live(5n, 3)
        live(5n, 2)

        // Longer than the window: live objects wait for the fill however late it opens
        await new Promise((resolve) => setTimeout(resolve, 150))

        fill([4, 0], [4, 1], [5, 0], [5, 1])

        live(5n, 5)

        expect(await read(7)).toEqual(['4.0', '4.1', '5.0', '5.1', '5.2', '5.3', '5.5'])
        expect(gaps).toEqual(['5.4-5.5'])

        await client.disconnect()
      })

      it("leaves a request with 'none' unordered, whatever the client default", async () => {
        const { client, transport } = await connected({}, { ordering: { window: { durationMs: 20 } } })
        const { live, read } = await subscribeOrdered(client, transport, 'none')

        // Ordered, both would wait for the start of group 5 and leave as 5.2, 5.3
        live(5n, 3)
        live(5n, 2)

        expect(await read(2)).toEqual(['5.3', '5.2'])

        await client.disconnect()
      })
    })

    it('keeps delivering the track a soft switch handed over from until it goes quiet', async () => {
      const { client, transport } = await connected()
      client.switchDrainTimeoutMs = 40

      const other = FullTrackName.tryNew('room/alice', 'video-low')
      const options = {
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      } as const

      const first = client.subscribe({ ...options, fullTrackName: ftn })

      const firstStream = await openedStream(transport, 0)
      firstStream.respond(subscribeOk(7n, {}, []))

      const firstId = ((await first) as { requestId: bigint }).requestId

      const switching = client.switch({
        switchFromRequestId: firstId,
        switchMode: SwitchMode.Soft,
        newSubscribeOptions: { ...options, fullTrackName: other },
      })

      const switchStream = await openedStream(transport, 1)
      // A soft switch asks for the PUBLISH_DONE that ends the drain it leaves behind.
      const { switchFrom } = (switchStream.messages[0] as Subscribe).parameters
      expect(switchFrom?.publishDone).toBe(true)

      switchStream.respond(subscribeOk(8n, {}, []))

      const target = client.requests.get(((await switching) as { requestId: bigint }).requestId) as SubscribeRequest
      const manager = target.manager!
      const outgoing = manager.stream.getReader()
      const old = client.requests.get(firstId) as SubscribeRequest

      // The handover, then the tail of the track being switched away from: it covers
      // the media up to where the new one starts, so it still belongs on the stream.
      manager.deliver(target, 'from-target' as unknown as MoqtObject)
      manager.deliver(old, 'from-drain' as unknown as MoqtObject)

      expect((await outgoing.read()).value).toBe('from-target')
      expect((await outgoing.read()).value).toBe('from-drain')

      outgoing.releaseLock()

      // Silence ends the drain, and the request is retired then.
      await vi.waitFor(() => expect(client.requests.has(firstId)).toBe(false))

      expect(streamResetCodeOf(firstStream.abortReason)).toBe(StreamResetCode.Cancelled)

      await client.disconnect()
    })

    it('orders a soft switch: the new track waits behind the drain, and no window expires meanwhile', async () => {
      const { client, transport } = await connected()
      client.switchDrainTimeoutMs = 60

      const other = FullTrackName.tryNew('room/alice', 'video-low')
      const options = {
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
        ordering: { window: { durationMs: 10 } },
      } as const

      const first = client.subscribe({ ...options, fullTrackName: ftn })

      const firstStream = await openedStream(transport, 0)
      firstStream.respond(subscribeOk(7n, {}, []))
      const firstId = ((await first) as { requestId: bigint }).requestId

      const switching = client.switch({
        switchFromRequestId: firstId,
        switchMode: SwitchMode.Soft,
        newSubscribeOptions: { ...options, fullTrackName: other },
      })
      ;(await openedStream(transport, 1)).respond(subscribeOk(8n, {}, []))

      const target = client.requests.get(((await switching) as { requestId: bigint }).requestId) as SubscribeRequest
      const old = client.requests.get(firstId) as SubscribeRequest
      const manager = target.manager!
      const out: string[] = []

      void (async () => {
        for await (const obj of manager.stream) out.push(`${obj.location.group}.${obj.location.object}`)
      })()

      const object = (name: FullTrackName, group: bigint, id: bigint) =>
        MoqtObject.newWithPayload(
          name,
          new Location(group, id),
          0,
          ObjectForwardingPreference.Subgroup,
          0,
          null,
          new Uint8Array([1]),
        )

      manager.deliver(old, object(ftn, 1n, 0n))
      manager.deliver(target, object(other, 2n, 0n)) // the handover: the old track drains
      manager.deliver(target, object(other, 2n, 2n)) // 2.1 is missing
      manager.deliver(old, object(ftn, 1n, 2n)) // 1.1 is late

      await new Promise((resolve) => setTimeout(resolve, 30))

      // Past the 10 ms window: the old track still waits for 1.1, the new one for the drain
      expect(out).toEqual(['1.0'])

      manager.deliver(old, object(ftn, 1n, 1n))

      await vi.waitFor(() => expect(out).toEqual(['1.0', '1.1', '1.2']))

      // The drain ends on silence: the new track follows, and its window expires as usual
      await vi.waitFor(() => expect(out).toEqual(['1.0', '1.1', '1.2', '2.0', '2.2']))

      await client.disconnect()
    })

    it('ends a drain on the PUBLISH_DONE that accounts for its streams', async () => {
      const { client, transport } = await connected()
      // Long enough that the backstop cannot be what ends this.
      client.switchDrainTimeoutMs = 60_000

      const other = FullTrackName.tryNew('room/alice', 'video-low')
      const options = {
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      } as const

      const first = client.subscribe({ ...options, fullTrackName: ftn })
      const firstStream = await openedStream(transport, 0)

      firstStream.respond(subscribeOk(7n, {}, []))

      const { requestId: firstId, done } = (await first) as SubscribeResult
      let doneWith: PublishDone | undefined | 'pending' = 'pending'
      void done.then((publishDone) => (doneWith = publishDone))

      const switching = client.switch({
        switchFromRequestId: firstId,
        switchMode: SwitchMode.Soft,
        newSubscribeOptions: { ...options, fullTrackName: other },
      })
      const switchStream = await openedStream(transport, 1)

      switchStream.respond(subscribeOk(8n, {}, []))

      const switched = (await switching) as SubscribeResult
      // The switch reports the chain's done, not just its own request's
      expect(switched.done).toBe(done)
      const target = client.requests.get(switched.requestId) as SubscribeRequest
      const old = client.requests.get(firstId) as SubscribeRequest

      target.manager!.deliver(target, {} as MoqtObject)

      // Its streams are already done when the count arrives, which is the ordinary
      // way round for a drain: the publisher stops, then says how much it sent.
      old.streamsAccepted = 2n

      firstStream.respond({
        kind: 'publishDone',
        statusCode: PublishDoneStatusCode.TrackEnded,
        streamCount: 2n,
        errorReason: '',
      })

      await vi.waitFor(() => expect(client.requests.has(firstId)).toBe(false))
      expect(firstStream.isClosed).toBe(true)
      expect(firstStream.abortReason).toBeUndefined()

      // The subscription is done only once the track it switched to is, with that PUBLISH_DONE
      await new Promise((resolve) => setTimeout(resolve))
      expect(doneWith).toBe('pending')

      target.streamsAccepted = 1n

      switchStream.respond({
        kind: 'publishDone',
        statusCode: PublishDoneStatusCode.Expired,
        streamCount: 1n,
        errorReason: '',
      })

      expect((await done)?.statusCode).toBe(PublishDoneStatusCode.Expired)

      await client.disconnect()
    })
  })
}
