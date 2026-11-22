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

import { ControlRecvStream, ControlSendStream, ControlStream } from '@/client/control_stream'
import { SubscribeRequest } from '@/client/request/subscribe'
import { isSessionClosed } from '@/client/util/stream_reset'
import { DEFAULT_MAX_FILTER_RANGES } from '@/client/types'
import type { EarlyDiscardPolicyConfig, MOQtailClientOptions } from '@/client/types'
import type { RequestStream } from '@/client/request_stream'
import { InternalError, MOQtailError, ProtocolViolationError } from '@/model/error/error'
import { resolveTransportUrl } from '@/model/common/moqt_url'
import { SUPPORTED_VERSIONS } from '@/model/constants'
import type { Setup } from '@/model/messages'
import { errorMessage } from '@/util/error'
import { logger } from '@/util/logger'
import { Datagrams } from './datagrams'
import { acceptIncomingBiStreams, acceptIncomingUniStreams, readControlStream } from './incoming'
import type { MOQtailClient } from './client'

/**
 * The state {@link MOQtailClient} keeps private, shared by the modules that implement it.
 * Never exported from the package.
 */
export class Session {
  /**
   * The bidirectional request stream each locally issued request runs on, keyed by the
   * requestId of the message that opened it. Responses and follow-ups arrive on it,
   * updates are written to it, and closing it is how the request is cancelled.
   */
  readonly requestStreams = new Map<bigint, RequestStream>()
  /**
   * Request ids of requests re-issued after a per-request GOAWAY, which consumes a fresh
   * Request ID each time (§10.1). The id the caller was given stays the client's key for
   * the request; this map translates the id now on the wire back to it.
   */
  readonly clientRequestIds = new Map<bigint, bigint>()
  /** Namespace path to the requestId that announced or subscribed to it, so it can be closed by namespace. */
  readonly namespaceRequestIds = new Map<string, bigint>()
  /**
   * Callbacks waiting for a track alias to be claimed, keyed by alias. Each belongs to a
   * data stream that arrived before the control message naming its alias.
   */
  readonly aliasWaiters = new Map<bigint, Set<() => void>>()
  readonly datagrams: Datagrams
  serverSetup!: Setup
  clientSetup!: Setup
  /** How many filter ranges this client allows the server per request, as advertised in SETUP. */
  maxFilterRanges = 0n
  /** Reader over incoming uni streams: the control recv half first, data streams after. */
  incomingUniStreams!: ReadableStreamDefaultReader<ReadableStream<Uint8Array>>
  destroyed = false
  /** {@link MOQtailClient.init} runs once, and only a `ready` client takes requests. */
  state: 'idle' | 'initializing' | 'ready' = 'idle'
  earlyDiscardPolicy: EarlyDiscardPolicyConfig | undefined
  #nextRequestId = 0n

  constructor(
    readonly client: MOQtailClient,
    readonly options: MOQtailClientOptions,
  ) {
    this.datagrams = new Datagrams(this)
  }

  /** The next client request id: this side's ids are even, so they advance by 2 (§10.1). */
  nextRequestId(): bigint {
    const id = this.#nextRequestId
    this.#nextRequestId += 2n

    return id
  }

  /** @throws MOQtailError when the client is destroyed or {@link MOQtailClient.init} has not completed. */
  ensureActive(): void {
    if (this.destroyed) throw new MOQtailError('MOQtailClient is destroyed and cannot be used.')
    if (this.state !== 'ready') throw new MOQtailError('MOQtailClient is not initialized. Await init() first.')
  }

  /** The session is going away, so a read or write failing is expected, not an error to log. */
  closing(error: unknown): boolean {
    return this.destroyed || isSessionClosed(error)
  }
}

/** Opens the WebTransport session and runs the SETUP handshake. */
export async function openSession(session: Session): Promise<void> {
  const { client, options } = session

  if (session.state !== 'idle' || session.destroyed) throw new MOQtailError('MOQtailClient.init may only run once.')

  session.state = 'initializing'

  // The protocols are sent in the wt-available-protocols header
  const transportOptions = {
    ...options.transportOptions,
    protocols: [...(options.transportOptions?.protocols ?? []), ...SUPPORTED_VERSIONS],
  }

  try {
    const { transportUrl } = resolveTransportUrl(options.url)
    client.webTransport = new WebTransport(transportUrl, transportOptions)

    await client.webTransport.ready

    if (options.dataStreamTimeoutMs) client.dataStreamTimeoutMs = options.dataStreamTimeoutMs
    if (options.controlStreamTimeoutMs) client.controlStreamTimeoutMs = options.controlStreamTimeoutMs
    client.trackAliasResolutionTimeoutMs = options.trackAliasResolutionTimeoutMs ?? client.trackAliasResolutionTimeoutMs
    client.switchDrainTimeoutMs = options.switchDrainTimeoutMs ?? client.switchDrainTimeoutMs
    client.fillOpenTimeoutMs = options.fillOpenTimeoutMs ?? client.fillOpenTimeoutMs

    // The control plane is a pair of uni streams. Open our send half and write SETUP first
    // so it goes out without waiting on the server's half, which the relay only opens after
    // accepting ours. Control streams get the highest priority.
    const sendStream = await client.webTransport.createUnidirectionalStream({ sendOrder: Number.MAX_SAFE_INTEGER })
    const sendHalf = new ControlSendStream(sendStream, client)

    // Range filters apply to what this client publishes, so it allows them unless the
    // caller set a limit of its own, 0 included.
    const maxFilterRanges = options.setupOptions?.maxFilterRanges ?? DEFAULT_MAX_FILTER_RANGES
    const setupOptions = { ...options.setupOptions, maxFilterRanges }
    if (setupOptions.authority !== undefined) {
      throw new ProtocolViolationError(
        'MOQtailClient.init',
        'AUTHORITY setup option MUST NOT be sent over WebTransport',
      )
    }

    session.maxFilterRanges = setupOptions.maxFilterRanges
    session.clientSetup = { kind: 'setup', options: setupOptions }
    await sendHalf.send(session.clientSetup)

    // The server's control stream is the first uni stream it opens; every later one is a
    // data stream, so the same reader serves acceptIncomingUniStreams.
    session.incomingUniStreams = client.webTransport.incomingUnidirectionalStreams.getReader()
    const { value: recvStream, done: recvStreamDone } = await session.incomingUniStreams.read()
    if (recvStreamDone || !recvStream) {
      throw new ProtocolViolationError('MOQtailClient.init', 'Session closed before the server control stream')
    }

    const recvHalf = new ControlRecvStream(recvStream, client.controlStreamTimeoutMs, client)
    client.controlStream = new ControlStream(sendHalf, recvHalf)
    const reader = client.controlStream.stream.getReader()
    const { value: response, done } = await reader.read()
    if (done) throw new ProtocolViolationError('MOQtailClient.init', 'Stream closed after client setup')
    if (response.kind !== 'setup') {
      throw new ProtocolViolationError('MOQtailClient.init', 'Expected setup as the first control message')
    }

    session.serverSetup = response

    reader.releaseLock()

    session.state = 'ready'

    void readControlStream(session)
    void acceptIncomingUniStreams(session)
    void acceptIncomingBiStreams(session)

    if (options.enableDatagrams) await session.datagrams.start()
  } catch (error) {
    await closeSession(session, new InternalError('MOQtailClient.init', errorMessage(error)))
    throw error
  }
}

/** Ends the session: every request stream closes and every open subscription ends. */
export async function closeSession(session: Session, reason?: unknown): Promise<void> {
  logger.log('MOQtailClient', 'disconnect', reason)

  if (session.destroyed) return
  session.destroyed = true

  if (session.state === 'idle') return

  const { client } = session

  await session.datagrams.stop()

  // Closing every request stream lets the peer see each request cancelled rather than
  // only the session going away.
  const openStreams = [...session.requestStreams.values()]
  session.requestStreams.clear()
  session.namespaceRequestIds.clear()
  await Promise.allSettled(openStreams.map((requestStream) => requestStream.close()))

  for (const request of client.requests.values()) {
    if (!(request instanceof SubscribeRequest)) continue

    ;(request.manager ?? request).objects.close()
    request.finish()
  }

  // close() does nothing on a session that is already closed or failed. A failed init()
  // may not have got as far as creating one.
  client.webTransport?.close()

  client.emit('sessionterminated', new InternalError('MOQtailClient.disconnect', errorMessage(reason)))
}

if (import.meta.vitest) {
  const { describe, it, expect, afterEach, vi } = import.meta.vitest
  const { MOQtailClient } = await import('@/client/client/client')
  const { MockWebTransport, connected, openedStream, restoreWebTransport, subscribeOk } =
    await import('@/client/testing')
  const { FrozenByteBuffer } = await import('@/model/common/byte_buffer')
  const { GroupOrder, LocationFilterType } = await import('@/model/constants')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { decodeMessage, encodeMessage } = await import('@/model/messages')
  const { locationFilter } = await import('@/model/parameters')
  type MockWebTransport = import('@/client/testing').MockWebTransport
  type SubscribeResult = import('@/client/types').SubscribeResult
  type Message = import('@/model/messages').Message

  describe('MOQtailClient control plane', () => {
    const originalWebTransport = globalThis.WebTransport

    afterEach(() => {
      globalThis.WebTransport = originalWebTransport
    })

    function connect(client = new MOQtailClient({ url: 'https://relay.example/moq' })): Promise<MOQtailClient> {
      globalThis.WebTransport = MockWebTransport as unknown as typeof WebTransport

      return client.init().then(() => client)
    }

    it('does no I/O until init(), and takes no requests before it', async () => {
      MockWebTransport.last = undefined as unknown as MockWebTransport
      const client = new MOQtailClient({ url: 'https://relay.example/moq' })

      expect(MockWebTransport.last).toBeUndefined()
      await expect(client.trackStatus(FullTrackName.tryNew('room', 'video'), 1n)).rejects.toThrow('not initialized')
    })

    it('lets a listener attached before init() see the SETUP it sends, and runs init() once', async () => {
      const client = new MOQtailClient({ url: 'https://relay.example/moq' })
      const sent: Message[] = []
      client.on('messagesent', (msg) => sent.push(msg))

      const connecting = connect(client)
      const transport = MockWebTransport.last

      await vi.waitFor(() => expect(transport.sentChunks).toHaveLength(1))

      transport.openIncomingUniStream(encodeMessage({ kind: 'setup', options: {} }))

      await connecting

      expect(sent[0]).toMatchObject({ kind: 'setup' })
      expect(client.clientSetup).toBe(sent[0])
      expect(client.serverSetup).toMatchObject({ kind: 'setup' })
      await expect(client.init()).rejects.toThrow('only run once')

      await client.disconnect()
    })

    it('handshakes over a pair of uni streams, SETUP first in both directions', async () => {
      const connecting = connect()
      const transport = MockWebTransport.last

      await vi.waitFor(() => expect(transport.sentChunks).toHaveLength(1))

      transport.openIncomingUniStream(encodeMessage({ kind: 'setup', options: {} }))

      const client = await connecting

      expect(transport.uniStreamOptions).toEqual([{ sendOrder: Number.MAX_SAFE_INTEGER }])
      expect(decodeMessage(new FrozenByteBuffer(transport.sentChunks[0]!))).toMatchObject({ kind: 'setup' })

      await client.disconnect()
    })

    it('rejects a peer control stream that does not begin with SETUP', async () => {
      const connecting = connect()
      const transport = MockWebTransport.last

      await vi.waitFor(() => expect(transport.sentChunks).toHaveLength(1))

      transport.openIncomingUniStream(
        encodeMessage({ kind: 'goAway', newSessionUri: 'https://elsewhere.example', timeout: 0n }),
      )

      await expect(connecting).rejects.toThrow('Expected setup as the first control message')
    })
  })

  describe('MOQtailClient session close', () => {
    const originalWebTransport = globalThis.WebTransport
    afterEach(() => {
      globalThis.WebTransport = originalWebTransport
    })

    it('closing with requests and streams in flight leaves no unhandled rejection and logs no error', async () => {
      globalThis.WebTransport = MockWebTransport as unknown as typeof WebTransport

      const unhandled: unknown[] = []
      const onUnhandled = (reason: unknown) => void unhandled.push(reason)
      process.on('unhandledRejection', onUnhandled)

      const errors = vi.spyOn(console, 'error').mockImplementation(() => {})

      try {
        const client = new MOQtailClient({ url: 'https://relay.example/moq' })
        const connecting = client.init()
        const transport = MockWebTransport.last

        await vi.waitFor(() => expect(transport.sentChunks).toHaveLength(1))

        transport.openIncomingUniStream(encodeMessage({ kind: 'setup', options: {} }))

        await connecting

        transport.openIncomingUniStream(new Uint8Array([0x10])) // a data stream cut inside its header
        transport.openIncomingBiStream() // a peer request with no message yet

        const subscribing = client.subscribe({
          fullTrackName: FullTrackName.tryNew('room', 'video'),
          locationFilter: locationFilter(LocationFilterType.NextObject),
          forward: true,
          groupOrder: GroupOrder.Original,
          priority: 0,
        })

        await vi.waitFor(() => expect(transport.biStreams).toHaveLength(1))

        await client.disconnect('test')

        await subscribing.catch(() => {}) // the caller's to handle

        await new Promise((resolve) => setTimeout(resolve, 10))

        expect(unhandled).toEqual([])
        expect(errors).not.toHaveBeenCalled()
      } finally {
        process.off('unhandledRejection', onUnhandled)

        errors.mockRestore()
      }
    })
  })

  describe('MOQtailClient session teardown', () => {
    restoreWebTransport()
    const ftn = FullTrackName.tryNew('room/alice', 'video')

    it('closes the WebTransport session on disconnect, once', async () => {
      const { client, transport } = await connected()

      await client.disconnect()

      await client.disconnect()

      expect(transport.closeCount).toBe(1)
    })

    it('ends a subscription stream when the session disconnects', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })

      ;(await openedStream(transport, 0)).respond(subscribeOk(7n, {}, []))

      const subscribed = (await subscribing) as SubscribeResult

      await client.disconnect()

      expect((await subscribed.stream.getReader().read()).done).toBe(true)
    })
  })
}
