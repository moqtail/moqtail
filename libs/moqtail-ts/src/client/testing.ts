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

/** A relay-free WebTransport for client tests. Never exported from the package. */

import { afterEach, expect, vi } from 'vitest'
import { MOQtailClient } from '@/client/client'
import type { MOQtailClientOptions } from '@/client/types'
import { ByteBuffer, FrozenByteBuffer } from '@/model/common/byte_buffer'
import { GroupOrder } from '@/model/constants'
import { decodeMessage, encodeMessage } from '@/model/messages'
import type { Message, SubscribeOk } from '@/model/messages'
import { FetchHeaderType, ObjectForwardingPreference } from '@/model/data/constant'
import { FetchObject } from '@/model/data/fetch_object'
import type { FetchObjectContext } from '@/model/data/fetch_object'
import { Header } from '@/model/data/header'
import type { MessageParameters, SetupOptions } from '@/model/parameters'
import type { TrackProperty } from '@/model/property/track_property'

/** A SUBSCRIBE_OK naming `trackAlias`. */
export function subscribeOk(
  trackAlias: bigint,
  parameters: MessageParameters = {},
  trackProperties: TrackProperty[] = [],
): SubscribeOk {
  return { kind: 'subscribeOk', trackAlias, parameters, trackProperties }
}

/** Restores the real WebTransport after each test of the calling suite. */
export function restoreWebTransport(): void {
  const original = globalThis.WebTransport

  afterEach(() => {
    globalThis.WebTransport = original
  })
}

/** A connected client whose handshake is already done, plus its peer control stream. */
export async function connected(
  serverOptions: SetupOptions = {},
  clientOptions: Partial<MOQtailClientOptions> = {},
): Promise<{
  client: MOQtailClient
  transport: MockWebTransport
  control: ReadableStreamDefaultController<Uint8Array>
}> {
  globalThis.WebTransport = MockWebTransport as unknown as typeof WebTransport

  const client = new MOQtailClient({ url: 'https://relay.example/moq', ...clientOptions })
  const connecting = client.init()
  const transport = MockWebTransport.last

  await vi.waitFor(() => expect(transport.sentChunks).toHaveLength(1))

  const control = transport.openIncomingUniStream(encodeMessage({ kind: 'setup', options: serverOptions }))

  await connecting

  return { client, transport, control }
}

/** Waits for the client to open its n-th bidi stream and write its first message. */
export async function openedStream(transport: MockWebTransport, index: number): Promise<MockBidiStream> {
  await vi.waitFor(() => {
    expect(transport.biStreams.length).toBeGreaterThan(index)
    expect(transport.biStreams[index]!.sentChunks.length).toBeGreaterThan(0)
  })

  return transport.biStreams[index]!
}

/** One bidirectional stream: what the client wrote, and a way to answer on it. */
/** The fetch stream bytes for `requestId` carrying `entries`, each encoded against the one before. */
export function fetchStreamBytes(requestId: bigint, entries: FetchObject[]): Uint8Array {
  const bytes = new ByteBuffer()
  bytes.putBytes(Header.newFetch(FetchHeaderType.Type0x05, requestId).serialize().toUint8Array())

  let ctx: FetchObjectContext | undefined = undefined

  for (const entry of entries) {
    bytes.putBytes(entry.serialize(ctx, GroupOrder.Ascending).toUint8Array())
    ctx = entry.nextContext(ctx)
  }

  return bytes.toUint8Array()
}

/** An Object of group `group` with id `id`, its payload the id. */
export const objectAt = (group: number, id: number) =>
  FetchObject.newObject(group, 0, id, 0, ObjectForwardingPreference.Subgroup, null, new Uint8Array([id]))

export class MockBidiStream {
  readonly sentChunks: Uint8Array[] = []
  readonly readable: ReadableStream<Uint8Array>
  readonly writable: WritableStream<Uint8Array>
  isClosed = false
  /** The reason the client aborted the send side with, if it did. */
  abortReason: unknown
  /** The reason the client cancelled the receive side with, if it did. */
  cancelReason: unknown
  #peer!: ReadableStreamDefaultController<Uint8Array>

  constructor() {
    this.readable = new ReadableStream<Uint8Array>({
      start: (controller) => {
        this.#peer = controller
      },
      cancel: (reason) => {
        this.cancelReason = reason
      },
    })
    this.writable = new WritableStream<Uint8Array>({
      write: (chunk) => {
        this.sentChunks.push(chunk)
      },
      close: () => {
        this.isClosed = true
      },
      abort: (reason) => {
        this.isClosed = true
        this.abortReason = reason
      },
    })
  }

  /** Delivers `msg` to the client on this stream, as the peer would. */
  respond(msg: Message): void {
    this.#peer.enqueue(encodeMessage(msg))
  }

  /** Errors the receive side, as a closing session does. */
  fail(reason: unknown): void {
    this.#peer.error(reason)
  }

  /** Everything the client has written to this stream so far. */
  get messages(): Message[] {
    return this.sentChunks.map((chunk) => decodeMessage(new FrozenByteBuffer(chunk)))
  }
}

export class MockWebTransport {
  static last: MockWebTransport
  readonly ready = Promise.resolve()
  readonly closed = new Promise<void>(() => {})
  readonly sentChunks: Uint8Array[] = []
  readonly uniStreamOptions: unknown[] = []
  /** Bidi streams the client opened, in order. */
  readonly biStreams: MockBidiStream[] = []
  readonly incomingBidirectionalStreams: ReadableStream<WebTransportBidirectionalStream>
  readonly incomingUnidirectionalStreams: ReadableStream<ReadableStream<Uint8Array>>
  #incoming!: ReadableStreamDefaultController<ReadableStream<Uint8Array>>
  #incomingBi!: ReadableStreamDefaultController<WebTransportBidirectionalStream>
  #incomingDatagrams!: ReadableStreamDefaultController<Uint8Array>
  readonly datagrams = {
    readable: new ReadableStream<Uint8Array>({
      start: (controller) => {
        this.#incomingDatagrams = controller
      },
    }),
    writable: new WritableStream<Uint8Array>(),
  }

  /** Delivers a datagram to the client, as the peer would. */
  receiveDatagram(bytes: Uint8Array): void {
    this.#incomingDatagrams.enqueue(bytes)
  }

  async createBidirectionalStream(): Promise<WebTransportBidirectionalStream> {
    const biStream = new MockBidiStream()
    this.biStreams.push(biStream)

    return biStream as unknown as WebTransportBidirectionalStream
  }

  /** Opens a peer-initiated bidi stream, as the relay would for a forwarded request. */
  openIncomingBiStream(): MockBidiStream {
    const biStream = new MockBidiStream()
    this.#incomingBi.enqueue(biStream as unknown as WebTransportBidirectionalStream)

    return biStream
  }

  constructor() {
    this.incomingBidirectionalStreams = new ReadableStream<WebTransportBidirectionalStream>({
      start: (controller) => {
        this.#incomingBi = controller
      },
    })
    this.incomingUnidirectionalStreams = new ReadableStream<ReadableStream<Uint8Array>>({
      start: (controller) => {
        this.#incoming = controller
      },
    })
    MockWebTransport.last = this
  }

  async createUnidirectionalStream(options?: unknown): Promise<WritableStream<Uint8Array>> {
    this.uniStreamOptions.push(options)

    return new WritableStream<Uint8Array>({
      write: (chunk) => {
        this.sentChunks.push(chunk)
      },
    })
  }

  /** Reasons the client cancelled peer uni streams with, in order. */
  readonly uniCancelReasons: unknown[] = []

  /**
   * Opens a peer uni stream carrying `bytes`, left open so the reader keeps waiting.
   * The returned controller writes more onto that same stream, which is how a second
   * control message reaches the client after the handshake.
   */
  openIncomingUniStream(bytes: Uint8Array): ReadableStreamDefaultController<Uint8Array> {
    let streamController!: ReadableStreamDefaultController<Uint8Array>

    this.#incoming.enqueue(
      new ReadableStream<Uint8Array>({
        start: (controller) => {
          streamController = controller
          this.#uniStreams.push(controller)
          controller.enqueue(bytes)
        },
        cancel: (reason) => {
          this.uniCancelReasons.push(reason)
        },
      }),
    )

    return streamController
  }

  /** How many times the client called `close()`. */
  closeCount = 0
  readonly #uniStreams: ReadableStreamDefaultController<Uint8Array>[] = []

  /** Errors every stream still open, as Chrome does when a session closes. */
  close(): void {
    this.closeCount++

    const closed = Object.assign(new Error('The session is closed.'), {
      name: 'WebTransportError',
      source: 'session',
    })

    for (const stream of [this.#incoming, this.#incomingBi, this.#incomingDatagrams, ...this.#uniStreams]) {
      stream.error(closed)
    }

    for (const stream of this.biStreams) stream.fail(closed)
  }
}
