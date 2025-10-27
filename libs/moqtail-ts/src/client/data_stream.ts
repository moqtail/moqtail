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
import { Header } from '../model/data/header'
import { NotEnoughBytesError, ProtocolViolationError, TimeoutError } from '../model/error/error'

export class SendStream {
  #lastObjectId?: bigint
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
    return new SendStream(header, writer, onDataSent)
  }

  async write(object: FetchObject | SubgroupObject): Promise<void> {
    const serializedObject = object.serialize(this.#lastObjectId).toUint8Array()
    this.#lastObjectId = object.objectId
    await this.#writer.write(serializedObject)
    if (this.onDataSent) this.onDataSent(object)
  }

  async close(): Promise<void> {
    if (this.#writer) {
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
    return new RecvStream(headerInstance, reader, internalBuffer, partialDataTimeout, onDataReceived)
  }

  async #ingestLoop(controller: ReadableStreamDefaultController<FetchObject | SubgroupObject>) {
    try {
      let previousObjectId: bigint | undefined = undefined
      while (true) {
        // Try to parse an object from buffer
        if (this.#internalBuffer.remaining > 0) {
          try {
            this.#internalBuffer.checkpoint()
            let object: FetchObject | SubgroupObject
            if (Header.isFetch(this.header)) {
              object = FetchObject.deserialize(this.#internalBuffer)
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
            controller.error(
              new ProtocolViolationError(
                'RecvStream',
                `Stream closed with incomplete object data. Remaining: ${this.#internalBuffer.remaining} bytes.`,
              ),
            )
          } else {
            controller.close()
          }
          break
        }
        if (value) {
          this.#internalBuffer.putBytes(value)
        }
      }
    } catch (error) {
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
        const fetchObject = FetchObject.newWithPayload(1, 1, 1, 1, null, payload)
        const reader = recvStream.stream.getReader()
        const receivePromise = reader.read()
        await sendStream.write(fetchObject)
        const { value: receivedObject } = await receivePromise
        expect(receivedObject).toEqual(fetchObject)
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
        testSubgroupHeader = Header.newSubgroup(SubgroupHeaderType.Type0x10, 0n, 0n, 0, 0)
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
