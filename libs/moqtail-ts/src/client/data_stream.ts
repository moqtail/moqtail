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
import { ObjectForwardingPreference } from '../model/data/constant'
import { Header } from '../model/data/header'
import { NotEnoughBytesError, ProtocolViolationError, TimeoutError } from '../model/error/error'
import { logger } from '../util/logger'

export class SendStream {
  #lastObjectId?: bigint
  #fetchPrevCtx?: FetchObjectContext
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>
  readonly onDataSent?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void
  private constructor(
    readonly header: Header,
    writer: WritableStreamDefaultWriter<Uint8Array>,
    onDataSent?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void,
  ) {
    if (onDataSent) this.onDataSent = onDataSent
    this.#writer = writer
  }

  static async new(
    writeStream: WritableStream<Uint8Array>,
    header: Header,
    onDataSent?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void,
  ): Promise<SendStream> {
    const writer = writeStream.getWriter()
    const serializedHeader = header.serialize().toUint8Array()
    await writer.write(serializedHeader)
    if (onDataSent) onDataSent(header)
    logger.debug('data_stream', `SendStream opened type=${header.type}`)
    return new SendStream(header, writer, onDataSent)
  }

  async write(object: FetchObject | SubgroupObject): Promise<void> {
    let serializedObject: Uint8Array
    if (object instanceof FetchObject) {
      serializedObject = object.serialize(this.#fetchPrevCtx).toUint8Array()
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

    await this.#writer.write(serializedObject)
    if (this.onDataSent) this.onDataSent(object)
  }

  async close(): Promise<void> {
    if (this.#writer) {
      logger.debug('data_stream', `SendStream closed type=${this.header.type}`)
      await this.#writer.close()
    }
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
  readonly onDataReceived?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void
  private constructor(
    readonly header: Header,
    reader: ReadableStreamDefaultReader<Uint8Array>,
    internalBuffer: ByteBuffer,
    partialDataTimeout?: number,
    onDataReceived?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void,
  ) {
    this.#reader = reader
    this.#internalBuffer = internalBuffer
    this.#partialDataTimeout = partialDataTimeout
    if (onDataReceived) this.onDataReceived = onDataReceived
    this.stream = new ReadableStream<FetchObject | SubgroupObject>({
      start: (controller) => this.#ingestLoop(controller),
      cancel: () => this.#reader.cancel(),
    })
  }

  static async new(
    readStream: ReadableStream<Uint8Array>,
    partialDataTimeout?: number,
    onDataReceived?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void,
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
          if (onDataReceived) onDataReceived(headerInstance)
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
      // Cleanup on error
      await reader.cancel(error).catch(() => {})
      reader.releaseLock()
      throw error
    }
    if (onDataReceived) onDataReceived(headerInstance)
    logger.debug('data_stream', `RecvStream opened type=${headerInstance.type}`)
    return new RecvStream(headerInstance, reader, internalBuffer, partialDataTimeout, onDataReceived)
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
              object = FetchObject.deserialize(this.#internalBuffer, fetchPrevCtx)
              const newCtx = object.toContext()
              if (newCtx) fetchPrevCtx = newCtx
            } else {
              object = SubgroupObject.deserialize(
                this.#internalBuffer,
                SubgroupHeaderType.hasExtensions(this.header.type),
                previousObjectId,
              )
              previousObjectId = object.objectId
            }
            this.#internalBuffer.commit()
            controller.enqueue(object)
            if (this.onDataReceived) this.onDataReceived(object)
            continue
          } catch (e) {
            if (e instanceof NotEnoughBytesError) {
              this.#internalBuffer.restore()
              // Fall through for reading  more data
            } else {
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
            controller.error(
              new ProtocolViolationError(
                'RecvStream',
                `Stream closed with incomplete object data. Remaining: ${this.#internalBuffer.remaining} bytes.`,
              ),
            )
          } else {
            logger.debug('data_stream', `RecvStream closed type=${this.header.type}`)
            controller.close()
          }
          break
        }
        if (value) {
          this.#internalBuffer.putBytes(value)
        }
      }
    } catch (error) {
      logger.error('data_stream', 'RecvStream ingest loop error', error)
      // Cleanup on error
      await this.#reader.cancel(error).catch(() => {})
      controller.error(error)
    }
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
}
