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

import { SendDatagramStream } from '@/client/datagram_stream'
import { SubscribeRequest } from '@/client/request/subscribe'
import { FrozenByteBuffer } from '@/model/common/byte_buffer'
import { ObjectDatagramType } from '@/model/data/constant'
import { Datagram } from '@/model/data/datagram'
import { FullTrackName } from '@/model/data/full_track_name'
import { MoqtObject } from '@/model/data/object'
import { MOQtailError } from '@/model/error/error'
import { logger } from '@/util/logger'
import type { Session } from './session'

/** Object datagrams of the session: one receive loop, per-track handlers, and the senders. */
export class Datagrams {
  /**
   * Every object received in a datagram, across all tracks. Only one reader may be
   * active; per-track handlers come from {@link Datagrams.onTrack}.
   */
  readonly objects: ReadableStream<MoqtObject>
  readonly #session: Session
  readonly #trackHandlers = new Map<bigint, (obj: MoqtObject) => void>()
  #receiving = false
  #reader: ReadableStreamDefaultReader<Uint8Array> | undefined
  #writer: WritableStreamDefaultWriter<Uint8Array> | undefined
  #objects?: ReadableStreamDefaultController<MoqtObject>

  constructor(session: Session) {
    this.#session = session
    this.objects = new ReadableStream<MoqtObject>({
      start: (controller) => {
        this.#objects = controller
      },
      cancel: () => this.stop(),
    })
  }

  get receiving(): boolean {
    return this.#receiving
  }

  async start(): Promise<void> {
    this.#session.ensureActive()

    if (this.#receiving) {
      logger.warn('MOQtailClient', 'Datagrams already started')
      return
    }

    const datagrams = this.#session.client.webTransport.datagrams as WebTransportDatagramDuplexStream & {
      createWritable?: () => WritableStream<Uint8Array>
    }
    this.#reader = datagrams.readable.getReader()
    // Safari has only createWritable(); other browsers still have the writable attribute.
    this.#writer = (datagrams.writable ?? datagrams.createWritable!()).getWriter()
    this.#receiving = true
    void this.#receive()
  }

  /** Idempotent. */
  async stop(): Promise<void> {
    if (!this.#receiving) return

    this.#receiving = false
    this.#trackHandlers.clear()

    if (this.#reader) {
      await this.#reader.cancel().catch(() => {})
      this.#reader.releaseLock()
      this.#reader = undefined
    }

    if (this.#writer) {
      await this.#writer.close().catch(() => {})
      this.#writer = undefined
    }

    this.#closeObjects()
  }

  /** Runs `handler` for each datagram object of `trackAlias`; returns the call that removes it. */
  onTrack(trackAlias: bigint, handler: (obj: MoqtObject) => void): () => void {
    this.#trackHandlers.set(trackAlias, handler)

    return () => this.#trackHandlers.delete(trackAlias)
  }

  offTrack(trackAlias: bigint): void {
    this.#trackHandlers.delete(trackAlias)
  }

  sender(trackAlias: bigint): SendDatagramStream {
    return SendDatagramStream.fromWriter(this.#writerOrThrow(), trackAlias, this.#session.client)
  }

  async send(trackAlias: bigint, object: MoqtObject): Promise<void> {
    const writer = this.#writerOrThrow()
    const datagram = object.tryIntoDatagram(trackAlias)
    const serialized = datagram.serialize().toUint8Array()

    this.#session.client.emit('datagramsent', datagram)
    await writer.write(serialized)
  }

  #writerOrThrow(): WritableStreamDefaultWriter<Uint8Array> {
    this.#session.ensureActive()

    if (!this.#writer) {
      throw new MOQtailError(
        'Datagrams not started. Call startDatagrams() first or set enableDatagrams: true in options.',
      )
    }

    return this.#writer
  }

  async #receive(): Promise<void> {
    try {
      while (this.#receiving && this.#reader) {
        const { done, value } = await this.#reader.read()
        if (done) {
          this.#receiving = false
          this.#closeObjects()
          break
        }

        if (!value || value.length === 0) continue

        try {
          this.#deliver(Datagram.deserialize(new FrozenByteBuffer(value)))
        } catch (error) {
          // One corrupt or unknown datagram must not stop the loop
          logger.warn('MOQtailClient', 'Failed to parse datagram:', error)
        }
      }
    } catch (error) {
      logger.error('MOQtailClient', 'Datagram reception error:', error)

      try {
        this.#objects?.error(error)
      } catch {
        // Already errored or closed
      }

      this.#receiving = false
    }
  }

  #deliver(datagram: Datagram): void {
    const { client } = this.#session
    const { trackAlias } = datagram

    client.emit('datagramreceived', datagram)

    const fullTrackName =
      client.aliasFullTrackNameMap.get(trackAlias) ?? FullTrackName.tryNew('unknown', `track-${trackAlias}`)
    const moqtObject = MoqtObject.fromDatagram(datagram, fullTrackName)

    const handler = this.#trackHandlers.get(trackAlias)

    try {
      handler?.(moqtObject)
    } catch (handlerError) {
      logger.warn('MOQtailClient', 'Datagram track handler error:', handlerError)
    }

    // An ordered subscription takes its datagrams too; unordered ones never did
    const holder = client.subscriptions.get(trackAlias)
    if (holder instanceof SubscribeRequest && holder.orderer) {
      if (!holder.largestLocation || holder.largestLocation.compare(moqtObject.location) < 0) {
        holder.largestLocation = moqtObject.location
      }

      const delivered = holder.manager.deliver(holder, moqtObject)
      if (delivered && ObjectDatagramType.isEndOfGroup(datagram.type)) {
        holder.orderer.endGroup(moqtObject.location.group, moqtObject.location.object)
      }
    }

    try {
      this.#objects?.enqueue(moqtObject)
    } catch {
      // The stream is closed
    }
  }

  #closeObjects(): void {
    try {
      this.#objects?.close()
    } catch {
      // Already closed
    }
  }
}

if (import.meta.vitest) {
  const { describe, it, expect, vi } = import.meta.vitest
  const { connected, openedStream, restoreWebTransport, subscribeOk } = await import('@/client/testing')
  const { Location } = await import('@/model/common/location')
  const { GroupOrder, LocationFilterType } = await import('@/model/constants')
  const { ObjectForwardingPreference } = await import('@/model/data/constant')
  const { locationFilter } = await import('@/model/parameters')

  describe('MOQtailClient datagrams', () => {
    restoreWebTransport()

    const ftn = FullTrackName.tryNew('room/alice', 'video')

    it('names a received datagram after the track its alias was claimed for', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const stream = await openedStream(transport, 0)
      stream.respond(subscribeOk(7n, {}, []))
      await subscribing

      await client.startDatagrams()

      const received: MoqtObject[] = []
      client.subscribeToTrackDatagrams(7n, (obj) => received.push(obj))
      transport.receiveDatagram(
        Datagram.newPayload(7n, 0n, 0n, null, null, new Uint8Array([1]))
          .serialize()
          .toUint8Array(),
      )

      await vi.waitFor(() => expect(received).toHaveLength(1))
      expect(received[0]!.fullTrackName.toString()).toBe(ftn.toString())

      await client.disconnect()
    })

    it('sends datagrams through createWritable() when the transport has no writable', async () => {
      const { client, transport } = await connected()
      const written: Uint8Array[] = []
      const sink = new WritableStream<Uint8Array>({ write: (chunk) => void written.push(chunk) })
      Object.assign(transport, {
        datagrams: { readable: transport.datagrams.readable, writable: undefined, createWritable: () => sink },
      })

      await client.startDatagrams()
      await client.sendDatagram(
        7n,
        MoqtObject.newWithPayload(
          ftn,
          new Location(0n, 0n),
          0,
          ObjectForwardingPreference.Datagram,
          null,
          null,
          new Uint8Array([1]),
        ),
      )

      expect(written).toHaveLength(1)
      await client.disconnect()
    })
  })
}
