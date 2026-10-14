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

import { ByteBuffer } from '../model/common/byte_buffer'
import {
  FetchHeader,
  FetchHeaderType,
  FetchObject,
  SubgroupHeader,
  SubgroupHeaderType,
  SubgroupObject,
} from '../model/data'
import { FetchObjectContext } from '../model/data/fetch_object'
import { GroupOrder } from '../model/control/constant'
import { ObjectForwardingPreference } from '../model/data/constant'
import { Header } from '../model/data/header'
import { NotEnoughBytesError, ProtocolViolationError, TimeoutError } from '../model/error/error'
import { StreamResetCode } from '../model/error/stream_reset'
import { asStreamResetError, streamResetCodeOf, streamResetReason } from './util/stream_reset'
import { logger } from '../util/logger'
import { DataStreamObserver, DataStreamReporter } from './data_event'

export class SendStream {
  #lastObjectId?: bigint
  #fetchPrevCtx?: FetchObjectContext
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>
  readonly #groupOrder: GroupOrder
  readonly #reporter: DataStreamReporter
  readonly onDataSent?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void
  private constructor(
    readonly header: Header,
    writer: WritableStreamDefaultWriter<Uint8Array>,
    reporter: DataStreamReporter,
    onDataSent?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void,
    groupOrder: GroupOrder = GroupOrder.Original,
  ) {
    if (onDataSent) this.onDataSent = onDataSent
    this.#writer = writer
    this.#reporter = reporter
    this.#groupOrder = groupOrder
  }

  /** @param observer - Receives a {@link DataEvent} for the header, each object, and the stream's end. */
  static async new(
    writeStream: WritableStream<Uint8Array>,
    header: Header,
    onDataSent?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void,
    groupOrder: GroupOrder = GroupOrder.Original,
    observer?: DataStreamObserver,
  ): Promise<SendStream> {
    const writer = writeStream.getWriter()
    const serializedHeader = header.serialize().toUint8Array()
    await writer.write(serializedHeader)
    if (onDataSent) onDataSent(header)
    const reporter = new DataStreamReporter('out', observer)
    reporter.header(header)
    logger.debug('data_stream', `SendStream opened type=${header.type}`)
    return new SendStream(header, writer, reporter, onDataSent, groupOrder)
  }

  async write(object: FetchObject | SubgroupObject): Promise<void> {
    let serializedObject: Uint8Array
    if (object instanceof FetchObject) {
      serializedObject = object.serialize(this.#fetchPrevCtx, this.#groupOrder).toUint8Array()
      const newCtx = object.toContext()
      if (newCtx) this.#fetchPrevCtx = newCtx
    } else {
      if (this.#lastObjectId !== undefined && object.objectId <= this.#lastObjectId) {
        logger.error(
          'data_stream',
          `SendStream out-of-order objectId=${object.objectId} lastObjectId=${this.#lastObjectId}`,
        )
        throw new Error(
          `SendStream.write: Out-of-order object detected. ` +
            `Attempted to write objectId=${object.objectId}, but lastObjectId=${this.#lastObjectId}.`,
        )
      }
      serializedObject = object.serialize(this.#lastObjectId).toUint8Array()
      this.#lastObjectId = object.objectId
    }

    try {
      await this.#writer.write(serializedObject)
    } catch (error) {
      this.#reporter.end('error')
      throw error
    }
    if (this.onDataSent) this.onDataSent(object)
    this.#reporter.object(this.header, object)
  }

  async close(): Promise<void> {
    if (this.#writer) {
      logger.debug('data_stream', `SendStream closed type=${this.header.type}`)
      await this.#writer.close()
      this.#reporter.end('fin')
    }
  }

  /** Abruptly tears the stream down, so the peer reads back `code` (§3.3.3). */
  async reset(code: StreamResetCode): Promise<void> {
    logger.debug('data_stream', `SendStream reset type=${this.header.type} code=${code}`)
    await this.#writer.abort(streamResetReason(code)).catch(() => {})
    this.#reporter.end('reset', code)
  }
}

function withTimeout<T>(promise: Promise<T>, ms?: number, errorMsg?: string): Promise<T> {
  if (ms === undefined) return promise
  let timeoutId: ReturnType<typeof setTimeout>
  const timeoutPromise = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(() => reject(new TimeoutError(errorMsg ?? `Timeout after ${ms}ms`)), ms)
  })
  return Promise.race([promise, timeoutPromise]).finally(() => clearTimeout(timeoutId))
}

export class RecvStream {
  readonly stream: ReadableStream<FetchObject | SubgroupObject>
  readonly #partialDataTimeout: number | undefined
  readonly #reader: ReadableStreamDefaultReader<Uint8Array>
  readonly #internalBuffer: ByteBuffer
  readonly #groupOrder: GroupOrder
  readonly #reporter: DataStreamReporter
  readonly onDataReceived?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void
  private constructor(
    readonly header: Header,
    reader: ReadableStreamDefaultReader<Uint8Array>,
    internalBuffer: ByteBuffer,
    reporter: DataStreamReporter,
    partialDataTimeout?: number,
    onDataReceived?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void,
    groupOrder: GroupOrder = GroupOrder.Original,
  ) {
    this.#reader = reader
    this.#reporter = reporter
    this.#internalBuffer = internalBuffer
    this.#partialDataTimeout = partialDataTimeout
    this.#groupOrder = groupOrder
    if (onDataReceived) this.onDataReceived = onDataReceived
    this.stream = new ReadableStream<FetchObject | SubgroupObject>({
      start: (controller) => this.#ingestLoop(controller),
      // Forwarded, not dropped: the reason is what carries the reset code to the peer.
      cancel: (reason) => this.#reader.cancel(reason),
    })
  }

  /**
   * @param resolveGroupOrder - Consulted once the FETCH header names its request, since
   *   Group ID deltas on the stream are signed by the Group Order the request negotiated.
   * @param observer - Receives a {@link DataEvent} for the header, each object, and the stream's end.
   */
  static async new(
    readStream: ReadableStream<Uint8Array>,
    partialDataTimeout?: number,
    onDataReceived?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void,
    resolveGroupOrder?: (header: Header) => GroupOrder,
    observer?: DataStreamObserver,
  ): Promise<RecvStream> {
    const reader = readStream.getReader()
    const internalBuffer = new ByteBuffer()
    let headerInstance: Header
    try {
      while (true) {
        let readResult: ReadableStreamReadResult<Uint8Array>

        if (partialDataTimeout !== undefined) {
          readResult = await withTimeout(
            reader.read(),
            partialDataTimeout,
            `RecvStream.new: Timeout after ${partialDataTimeout}ms waiting for header data`,
          )
        } else {
          readResult = await reader.read()
        }

        const { done, value } = readResult
        if (done) {
          throw new ProtocolViolationError(
            'RecvStream.new',
            internalBuffer.length > 0
              ? 'Stream closed with incomplete header data.'
              : 'Stream closed before any header data received.',
          )
        }
        if (value) {
          internalBuffer.putBytes(value)
        }
        try {
          internalBuffer.checkpoint()
          headerInstance = Header.deserialize(internalBuffer)
          internalBuffer.commit()
          break
        } catch (e) {
          if (e instanceof NotEnoughBytesError) {
            internalBuffer.restore()
            continue
          } else {
            throw e
          }
        }
      }
    } catch (error) {
      await reader.cancel(streamResetReason(StreamResetCode.InternalError)).catch(() => {})
      reader.releaseLock()
      throw asStreamResetError('RecvStream.new', error)
    }
    if (onDataReceived) onDataReceived(headerInstance)
    const reporter = new DataStreamReporter('in', observer)
    reporter.header(headerInstance)
    logger.debug('data_stream', `RecvStream opened type=${headerInstance.type}`)
    // Resolved before the ingest loop starts: the first object may already be buffered.
    const groupOrder =
      Header.isFetch(headerInstance) && resolveGroupOrder ? resolveGroupOrder(headerInstance) : GroupOrder.Original
    return new RecvStream(
      headerInstance,
      reader,
      internalBuffer,
      reporter,
      partialDataTimeout,
      onDataReceived,
      groupOrder,
    )
  }

  async #ingestLoop(controller: ReadableStreamDefaultController<FetchObject | SubgroupObject>) {
    try {
      let previousObjectId: bigint | undefined = undefined
      let fetchPrevCtx: FetchObjectContext | undefined = undefined
      while (true) {
        // Try to parse an object from buffer
        if (this.#internalBuffer.remaining > 0) {
          try {
            this.#internalBuffer.checkpoint()
            let object: FetchObject | SubgroupObject
            if (Header.isFetch(this.header)) {
              object = FetchObject.deserialize(this.#internalBuffer, fetchPrevCtx, this.#groupOrder)
              const newCtx = object.toContext()
              if (newCtx) fetchPrevCtx = newCtx
            } else {
              object = SubgroupObject.deserialize(
                this.#internalBuffer,
                SubgroupHeaderType.hasProperties(this.header.type),
                previousObjectId,
              )
              // The first object names the subgroup when the header type defers its
              // ID; do it before the object is handed on, so whoever routes it sees
              // a header that identifies the subgroup it belongs to.
              this.header.resolveSubgroupId(object.objectId)
              previousObjectId = object.objectId
            }
            this.#internalBuffer.commit()
            controller.enqueue(object)
            if (this.onDataReceived) this.onDataReceived(object)
            this.#reporter.object(this.header, object)
            continue
          } catch (e) {
            if (e instanceof NotEnoughBytesError) {
              this.#internalBuffer.restore()
              // Fall through for reading  more data
            } else {
              this.#reporter.end('error')
              controller.error(e)
              break
            }
          }
        }
        let readResult: ReadableStreamReadResult<Uint8Array>

        if (this.#partialDataTimeout) {
          readResult = await withTimeout(
            this.#reader.read(),
            this.#partialDataTimeout,
            `RecvStream: Timeout after ${this.#partialDataTimeout}ms waiting for object data`,
          )
        } else {
          readResult = await this.#reader.read()
        }

        const { done, value } = readResult
        if (done) {
          if (this.#internalBuffer.remaining > 0) {
            logger.error(
              'data_stream',
              `RecvStream closed with incomplete data remaining=${this.#internalBuffer.remaining}`,
            )
            this.#reporter.end('error')
            controller.error(
              new ProtocolViolationError(
                'RecvStream',
                `Stream closed with incomplete object data. Remaining: ${this.#internalBuffer.remaining} bytes.`,
              ),
            )
          } else {
            logger.debug('data_stream', `RecvStream closed type=${this.header.type}`)
            this.#reporter.end('fin')
            controller.close()
          }
          break
        }
        if (value) {
          this.#internalBuffer.putBytes(value)
        }
      }
    } catch (error) {
      const peerReset = streamResetCodeOf(error)
      if (peerReset !== undefined) {
        // The peer reset the stream. That is an ordinary way for one to end.
        logger.debug('data_stream', `RecvStream reset by peer code=${StreamResetCode[peerReset]}`)
        this.#reporter.end('reset', peerReset)
      } else {
        logger.error('data_stream', 'RecvStream ingest loop error', error)
        this.#reporter.end('error')
        await this.#reader.cancel(streamResetReason(StreamResetCode.InternalError)).catch(() => {})
      }
      // Either way the consumer is told, so a half-read stream is never mistaken for
      // one that ended cleanly.
      controller.error(asStreamResetError('RecvStream', error))
    }
  }

  /** Asks the peer to stop sending on this stream, reporting `code` (§3.3.3). */
  async stopSending(code: StreamResetCode): Promise<void> {
    logger.debug('data_stream', `RecvStream stopSending type=${this.header.type} code=${code}`)
    await this.#reader.cancel(streamResetReason(code)).catch(() => {})
  }
}

if (import.meta.vitest) {
  const { describe, test, beforeEach, expect } = import.meta.vitest

  describe('DataStream', () => {
    let sendStream: SendStream
    let recvStream: RecvStream
    let testSubgroupHeader: Header

    describe('Fetch', () => {
      beforeEach(async () => {
        testSubgroupHeader = Header.newFetch(FetchHeaderType.Type0x05, 5n)
        const transport = new TransformStream<Uint8Array, Uint8Array>(
          {},
          { highWaterMark: 16 * 1024 },
          { highWaterMark: 16 * 1024 },
        )
        const sendStreamPromise = SendStream.new(transport.writable, testSubgroupHeader)
        const recvStreamPromise = RecvStream.new(transport.readable)
        sendStream = await sendStreamPromise
        recvStream = await recvStreamPromise
      })
      test('Header is correctly received after sending', () => {
        expect(recvStream.header).toEqual(testSubgroupHeader)
      })

      test('Full object roundtrip', async () => {
        const payload = new Uint8Array([1, 2, 3, 4, 5])
        const fetchObject = FetchObject.newObject(1, 1, 1, 1, ObjectForwardingPreference.Subgroup, null, payload)
        const reader = recvStream.stream.getReader()
        const receivePromise = reader.read()
        await sendStream.write(fetchObject)
        const { value: receivedObject } = await receivePromise
        expect(receivedObject).toBeInstanceOf(FetchObject)
        const received = receivedObject as FetchObject
        expect(received.groupId).toEqual(fetchObject.groupId)
        expect(received.subgroupId).toEqual(fetchObject.subgroupId)
        expect(received.objectId).toEqual(fetchObject.objectId)
        expect(received.payload).toEqual(fetchObject.payload)
        reader.releaseLock()
      })

      test('Stream completion: nextObject returns null after publisher closes', async () => {
        await sendStream.close()
        const reader = recvStream.stream.getReader()
        const receivedObject = await reader.read()
        expect(receivedObject.done).toBeTruthy()
        reader.releaseLock()
      })

      test('nextObject returns null if called again after stream completion', async () => {
        await sendStream.close()
        const reader = recvStream.stream.getReader()
        await reader.read()
        const receivedObjectAgain = await reader.read()
        expect(receivedObjectAgain.done).toBeTruthy()
        reader.releaseLock()
      })
    })

    describe('Subgroup', () => {
      beforeEach(async () => {
        testSubgroupHeader = Header.newSubgroup(SubgroupHeaderType.fromProperties(false, 0, false), 0n, 0n, 0, 0)
        const transport = new TransformStream<Uint8Array, Uint8Array>()
        const sendStreamPromise = SendStream.new(transport.writable, testSubgroupHeader)
        const recvStreamPromise = RecvStream.new(transport.readable)
        sendStream = await sendStreamPromise
        recvStream = await recvStreamPromise
      })
      test('Header is correctly received after sending', () => {
        expect(recvStream.header).toEqual(testSubgroupHeader)
      })

      test('Full object roundtrip', async () => {
        const payload = new Uint8Array([1, 2, 3, 4, 5])
        const fetchObject = SubgroupObject.newWithPayload(1, null, payload)
        const reader = recvStream.stream.getReader()
        const receivePromise = reader.read()
        await sendStream.write(fetchObject)
        const { value: receivedObject } = await receivePromise
        expect(receivedObject).toEqual(fetchObject)
        reader.releaseLock()
      })
    })
  })
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SendStream', () => {
    test('write cleanly rejects out-of-order objects', async () => {
      const writeStream = new WritableStream<Uint8Array>({
        write(_chunk) {},
      })

      const dummyHeader = {
        serialize: () => ({ toUint8Array: () => new Uint8Array() }),
      }
      const sendStream = await SendStream.new(writeStream, dummyHeader as any)
      const dummySerialized = { toUint8Array: () => new Uint8Array() }

      await expect(
        sendStream.write({
          objectId: 10n,
          serialize: () => dummySerialized,
        } as any),
      ).resolves.not.toThrow()

      await expect(
        sendStream.write({
          objectId: 9n,
          serialize: () => dummySerialized,
        } as any),
      ).rejects.toThrow(/Out-of-order object detected/)

      await sendStream.close()
    })
  })

  describe('stream reset codes', () => {
    const header = () => Header.newFetch(FetchHeaderType.Type0x05, 5n)

    /** A SendStream over a writable that records what it was aborted with. */
    async function sender(): Promise<{ send: SendStream; reason: () => unknown }> {
      let reason: unknown
      const writable = new WritableStream<Uint8Array>({
        write: () => {},
        abort: (r) => {
          reason = r
        },
      })
      return { send: await SendStream.new(writable, header()), reason: () => reason }
    }

    /** A RecvStream over a readable that records what it was cancelled with. */
    async function receiver(): Promise<{ recv: RecvStream; reason: () => unknown }> {
      let reason: unknown
      const bytes = header().serialize().toUint8Array()
      const readable = new ReadableStream<Uint8Array>({
        start: (controller) => controller.enqueue(bytes),
        cancel: (r) => {
          reason = r
        },
      })
      return { recv: await RecvStream.new(readable), reason: () => reason }
    }

    test('SendStream.reset aborts with a numeric code the peer can read back', async () => {
      const { send, reason } = await sender()
      await send.reset(StreamResetCode.DeliveryTimeout)
      expect(streamResetCodeOf(reason())).toBe(StreamResetCode.DeliveryTimeout)
    })

    test('RecvStream.stopSending cancels with a numeric code', async () => {
      const { recv, reason } = await receiver()
      await recv.stopSending(StreamResetCode.TooFarBehind)
      expect(streamResetCodeOf(reason())).toBe(StreamResetCode.TooFarBehind)
    })

    test('cancelling the object stream forwards the code to the transport', async () => {
      const { recv, reason } = await receiver()
      await recv.stream.getReader().cancel(streamResetReason(StreamResetCode.DeliveryTimeout))
      expect(streamResetCodeOf(reason())).toBe(StreamResetCode.DeliveryTimeout)
    })
  })
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { ObjectStatus } = await import('../model/data/constant')
  type DataEvent = import('./data_event').DataEvent

  function observe() {
    const events: DataEvent[] = []
    return { events, observer: { onDataEvent: (e: DataEvent) => events.push(e) } }
  }

  async function pair(header: Header) {
    const sent = observe()
    const received = observe()
    const transport = new TransformStream<Uint8Array, Uint8Array>()
    const [send, recv] = await Promise.all([
      SendStream.new(transport.writable, header, undefined, undefined, sent.observer),
      RecvStream.new(transport.readable, undefined, undefined, undefined, received.observer),
    ])
    return { send, recv, sent: sent.events, received: received.events }
  }

  async function drain(recv: RecvStream) {
    const reader = recv.stream.getReader()
    try {
      while (!(await reader.read()).done);
    } catch {
      // A reset stream errors the reader; the events are what is under test.
    }
  }

  describe('data events', () => {
    test('report a subgroup stream from header to end on both sides', async () => {
      const header = Header.newSubgroup(SubgroupHeaderType.fromProperties(false, 0, false), 7n, 3n, 0, 0)
      const { send, recv, sent, received } = await pair(header)
      await send.write(SubgroupObject.newWithPayload(1, null, new Uint8Array(5)))
      await send.write(SubgroupObject.newWithStatus(2, null, ObjectStatus.EndOfGroup))
      await send.close()
      await drain(recv)

      for (const [events, direction] of [
        [sent, 'out'],
        [received, 'in'],
      ] as const) {
        expect(events.map((e) => e.kind)).toEqual(['header', 'object', 'object', 'end'])
        expect(new Set(events.map((e) => e.streamId)).size).toBe(1)
        expect(events[0]).toMatchObject({ direction, stream: 'subgroup', trackAlias: 7n, groupId: 3n })
        expect(events[1]).toMatchObject({ groupId: 3n, objectId: 1n, payloadLength: 5 })
        expect(events[1]).not.toHaveProperty('objectStatus')
        expect(events[2]).toMatchObject({ objectId: 2n, payloadLength: 0, objectStatus: ObjectStatus.EndOfGroup })
        expect(events[3]).toMatchObject({ reason: 'fin' })
      }
      expect(sent[0]!.streamId).not.toBe(received[0]!.streamId)
    })

    test('carry the LOC Timestamp and a monotonic local time', async () => {
      const { LOCProperties } = await import('../model/property/loc')
      const header = Header.newSubgroup(SubgroupHeaderType.fromProperties(true, 0, false), 1n, 0n, 0, 0)
      const { send, recv, sent, received } = await pair(header)
      await send.write(
        SubgroupObject.newWithPayload(1, new LOCProperties().addTimestamp(1234).build(), new Uint8Array(1)),
      )
      await send.write(SubgroupObject.newWithPayload(2, null, new Uint8Array(1)))
      await send.close()
      await drain(recv)
      for (const events of [sent, received]) {
        expect(events[1]).toMatchObject({ objectId: 1n, locTimestamp: 1234n })
        expect(events[2]).not.toHaveProperty('locTimestamp')
        const times = events.map((e) => e.at)
        expect(times).toEqual([...times].sort((a, b) => a - b))
      }
    })

    test('report fetch objects with their own location', async () => {
      const { send, recv, sent, received } = await pair(Header.newFetch(FetchHeaderType.Type0x05, 5n))
      await send.write(FetchObject.newObject(4, 1, 9, 1, ObjectForwardingPreference.Subgroup, null, new Uint8Array(3)))
      await send.close()
      await drain(recv)
      for (const events of [sent, received]) {
        expect(events[0]).toMatchObject({ stream: 'fetch', requestId: 5n })
        expect(events[1]).toMatchObject({ groupId: 4n, subgroupId: 1n, objectId: 9n, payloadLength: 3 })
        expect(events[2]).toMatchObject({ reason: 'fin' })
      }
    })

    test('report a reset with its code on both sides', async () => {
      const header = Header.newSubgroup(SubgroupHeaderType.fromProperties(false, 0, false), 1n, 0n, 0, 0)
      const { send, recv, sent, received } = await pair(header)
      const drained = drain(recv)
      await send.reset(StreamResetCode.DeliveryTimeout)
      await drained
      for (const events of [sent, received]) {
        expect(events.at(-1)).toMatchObject({
          kind: 'end',
          reason: 'reset',
          resetCode: StreamResetCode.DeliveryTimeout,
        })
      }
    })

    test('a throwing observer does not break the stream', async () => {
      const header = Header.newSubgroup(SubgroupHeaderType.fromProperties(false, 0, false), 1n, 0n, 0, 0)
      const transport = new TransformStream<Uint8Array, Uint8Array>()
      const throwing = {
        onDataEvent: () => {
          throw new Error('observer failed')
        },
      }
      const [send, recv] = await Promise.all([
        SendStream.new(transport.writable, header, undefined, undefined, throwing),
        RecvStream.new(transport.readable, undefined, undefined, undefined, throwing),
      ])
      const reader = recv.stream.getReader()
      const read = reader.read()
      await send.write(SubgroupObject.newWithPayload(1, null, new Uint8Array(2)))
      expect((await read).value).toBeInstanceOf(SubgroupObject)
    })
  })
}
