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

import { ControlMessage } from '../model/control/control_message'
import { FrozenByteBuffer, ByteBuffer } from '../model/common/byte_buffer'
import { NotEnoughBytesError, TerminationError, TimeoutError } from '../model/error/error'
import { TerminationCode } from '../model/error/constant'
import { SetupParameters, ClientSetup } from '@/model'
import { createLogger } from '../util/logger'

const logger = createLogger('control_stream')

function withTimeout<T>(promise: Promise<T>, ms?: number, errorMsg?: string): Promise<T> {
  if (ms === undefined) return promise
  let timeoutId: ReturnType<typeof setTimeout>
  const timeoutPromise = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(() => reject(new TimeoutError(errorMsg ?? `Timeout after ${ms}ms`)), ms)
  })
  return Promise.race([promise, timeoutPromise]).finally(() => clearTimeout(timeoutId))
}

export class ControlStream {
  readonly stream: ReadableStream<ControlMessage>
  #receiveBuffer: ByteBuffer
  #expectedPayloadLength: number | null = null
  #partialMessageTimeoutMs: number | undefined
  #reader: ReadableStreamDefaultReader<Uint8Array>
  #writer: WritableStreamDefaultWriter<Uint8Array>
  onMessageSent?: (msg: ControlMessage) => void
  onMessageReceived?: (msg: ControlMessage) => void

  private constructor(
    readStream: ReadableStream<Uint8Array>,
    writeStream: WritableStream<Uint8Array>,
    partialMessageTimeoutMs?: number,
    onMessageSent?: (msg: ControlMessage) => void,
    onMessageReceived?: (msg: ControlMessage) => void,
  ) {
    this.#receiveBuffer = new ByteBuffer()
    this.#partialMessageTimeoutMs = partialMessageTimeoutMs
    this.#reader = readStream.getReader()
    this.#writer = writeStream.getWriter()
    if (onMessageReceived) this.onMessageReceived = onMessageReceived
    if (onMessageSent) this.onMessageSent = onMessageSent
    this.stream = new ReadableStream<ControlMessage>({
      start: (controller) => this.#ingestLoop(controller),
      cancel: () => this.close(),
    })
  }

  static new(
    bidirectionalStream: WebTransportBidirectionalStream,
    partialMessageTimeoutMs?: number,
    onMessageSent?: (msg: ControlMessage) => void,
    onMessageReceived?: (msg: ControlMessage) => void,
  ): ControlStream {
    return new ControlStream(
      bidirectionalStream.readable,
      bidirectionalStream.writable,
      partialMessageTimeoutMs,
      onMessageSent,
      onMessageReceived,
    )
  }

  async send(message: ControlMessage): Promise<void> {
    const msgName = message.constructor.name
    try {
      const serializedMessage = ControlMessage.serialize(message)
      logger.debug(`send: waiting for writer.ready — ${msgName} (${serializedMessage.length}B)`)
      await this.#writer.ready
      logger.debug(`send: writer ready, writing — ${msgName}`)
      await this.#writer.write(serializedMessage.toUint8Array())
      logger.debug(`send: write complete — ${msgName}`)
      if (this.onMessageSent) this.onMessageSent(message)
    } catch (error: any) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(`send: failed to write ${msgName} — ${errorMessage}`)
      await this.close()
      throw new TerminationError(
        `ControlStream.send: Failed to write message: ${errorMessage}`,
        TerminationCode.INTERNAL_ERROR,
      )
    }
  }

  public async close(): Promise<void> {
    logger.debug('close: closing writer and cancelling reader')
    await Promise.allSettled([this.#writer.close().catch(() => {}), this.#reader.cancel().catch(() => {})])
    logger.debug('close: done')
  }

  async #ingestLoop(controller: ReadableStreamDefaultController<ControlMessage>) {
    logger.debug('ingestLoop: started')
    try {
      while (true) {
        if (this.#receiveBuffer.length === 0) {
          logger.debug('ingestLoop: buffer empty, awaiting reader.read()')
          const readResult = await this.#reader.read()
          this.#handleReadResult(readResult)
          if (readResult.done) {
            logger.debug('ingestLoop: reader done (stream closed by peer)')
            break
          }
          logger.debug(`ingestLoop: received ${readResult.value?.byteLength ?? 0}B chunk`)
          continue
        }
        try {
          this.#receiveBuffer.checkpoint()
          const startOffset = this.#receiveBuffer.offset
          this.#receiveBuffer.getVI()
          const payloadLength = this.#receiveBuffer.getU16()
          const headerSize = this.#receiveBuffer.offset - startOffset
          const totalMessageSize = headerSize + payloadLength
          this.#receiveBuffer.restore()
          if (this.#receiveBuffer.length >= totalMessageSize) {
            const messageBytes = this.#receiveBuffer.getBytes(totalMessageSize)
            this.#receiveBuffer.commit()
            this.#expectedPayloadLength = null
            const msg = ControlMessage.deserialize(new FrozenByteBuffer(messageBytes))
            logger.debug(`ingestLoop: deserialized ${msg.constructor.name} (${totalMessageSize}B)`)
            controller.enqueue(msg)
            if (this.onMessageReceived) this.onMessageReceived(msg)
            continue
          }
          this.#expectedPayloadLength = payloadLength
          logger.debug(
            `ingestLoop: partial message — have ${this.#receiveBuffer.length}B, need ${totalMessageSize}B, awaiting more data`,
          )
          const timeoutMessage = `ControlStream: Timeout waiting for partial message data (expected ${payloadLength} bytes)`
          let readResult
          if (this.#partialMessageTimeoutMs !== undefined) {
            readResult = await withTimeout(this.#reader.read(), this.#partialMessageTimeoutMs, timeoutMessage)
          } else {
            readResult = await this.#reader.read()
          }
          this.#handleReadResult(readResult as ReadableStreamReadResult<Uint8Array>)
          if ((readResult as ReadableStreamReadResult<Uint8Array>).done) {
            logger.debug('ingestLoop: reader done while waiting for partial message')
            break
          }
        } catch (error: any) {
          if (error instanceof NotEnoughBytesError) {
            logger.debug('ingestLoop: not enough bytes for header, awaiting more data')
            let readResult
            if (this.#partialMessageTimeoutMs !== undefined) {
              readResult = await withTimeout(
                this.#reader.read(),
                this.#partialMessageTimeoutMs,
                'ControlStream: Timeout waiting for message header',
              )
            } else {
              readResult = await this.#reader.read()
            }
            this.#handleReadResult(readResult as ReadableStreamReadResult<Uint8Array>)
            if ((readResult as ReadableStreamReadResult<Uint8Array>).done) {
              logger.debug('ingestLoop: reader done while waiting for header')
              break
            }
          } else {
            logger.error(`ingestLoop: deserialization error — ${error.message}`)
            controller.error(
              new TerminationError(
                `ControlStream: Deserialization error: ${error.message}`,
                TerminationCode.PROTOCOL_VIOLATION,
              ),
            )
            await this.close()
            break
          }
        }
      }
    } catch (error) {
      logger.error(`ingestLoop: unexpected error — ${error instanceof Error ? error.message : String(error)}`)
      controller.error(error)
      await this.close()
    } finally {
      logger.debug('ingestLoop: ended, closing controller')
      controller.close()
      await this.close()
    }
  }

  #handleReadResult(readResult: ReadableStreamReadResult<Uint8Array>): void {
    if (readResult.done) {
      if (this.#receiveBuffer.length > 0 || this.#expectedPayloadLength !== null) {
        logger.error(
          `handleReadResult: peer closed stream with incomplete data — bufferLen=${this.#receiveBuffer.length} expectedPayload=${this.#expectedPayloadLength}`,
        )
        throw new TerminationError(
          'ControlStream: Stream closed by peer with incomplete message data.',
          TerminationCode.PROTOCOL_VIOLATION,
        )
      }
      return
    }
    if (readResult.value) {
      this.#receiveBuffer.putBytes(readResult.value)
      logger.debug(
        `handleReadResult: appended ${readResult.value.byteLength}B — bufferLen=${this.#receiveBuffer.length}`,
      )
    }
  }
}

if (import.meta.vitest) {
  const { describe, it, expect, vi, beforeEach } = import.meta.vitest

  interface MockReadableStreamReader {
    read(): Promise<ReadableStreamReadResult<Uint8Array>>
    releaseLock(): void
  }

  interface MockWritableStreamWriter {
    ready: Promise<void>
    write(chunk: Uint8Array): Promise<void>
    releaseLock(): void
  }

  class MockReadableStream {
    private reader: MockReadableStreamReader | null = null
    private chunks: Uint8Array[] = []
    private closed = false
    private cancelled = false

    constructor(chunks: Uint8Array[] = []) {
      this.chunks = [...chunks]
    }

    getReader(): MockReadableStreamReader {
      if (this.reader) {
        throw new Error('Reader already acquired')
      }

      this.reader = {
        read: vi.fn().mockImplementation(async () => {
          if (this.cancelled) {
            return { done: true, value: undefined }
          }
          if (this.chunks.length > 0) {
            const value = this.chunks.shift()!
            return { done: false, value }
          }
          if (this.closed) {
            return { done: true, value: undefined }
          }
          // Simulate waiting for data indefinitely
          return new Promise(() => {})
        }),
        releaseLock: vi.fn().mockImplementation(() => {
          this.reader = null
        }),
      }

      return this.reader
    }

    async cancel(): Promise<void> {
      this.cancelled = true
      return Promise.resolve()
    }

    // Test helper methods
    addChunk(chunk: Uint8Array): void {
      this.chunks.push(chunk)
    }

    close(): void {
      this.closed = true
    }
  }

  class MockWritableStream {
    private writer: MockWritableStreamWriter | null = null
    private writtenData: Uint8Array[] = []

    getWriter(): MockWritableStreamWriter {
      if (this.writer) {
        throw new Error('Writer already acquired')
      }

      this.writer = {
        ready: Promise.resolve(),
        write: vi.fn().mockImplementation(async (chunk: Uint8Array) => {
          this.writtenData.push(new Uint8Array(chunk))
          return Promise.resolve()
        }),
        releaseLock: vi.fn().mockImplementation(() => {
          this.writer = null
        }),
      }

      return this.writer
    }

    async close(): Promise<void> {
      return Promise.resolve()
    }

    getWrittenData(): Uint8Array[] {
      return this.writtenData
    }
  }

  function createMockBidirectionalStream(readableChunks: Uint8Array[] = []): WebTransportBidirectionalStream {
    const readable = new MockReadableStream(readableChunks)
    const writable = new MockWritableStream()

    return {
      readable: readable as unknown as ReadableStream<Uint8Array>,
      writable: writable as unknown as WritableStream<Uint8Array>,
    }
  }
  describe('ControlStream', () => {
    describe('ClientSetup', () => {
      let controlStream: ControlStream
      let mockBidirectionalStream: WebTransportBidirectionalStream
      beforeEach(() => {
        vi.clearAllMocks()
      })
      it('should handle full message roundtrip', async () => {
        const setupParams = new SetupParameters()
          .addPath('/test/path')
          .addMaxRequestId(1000n)
          .addMaxAuthTokenCacheSize(500n)
          .build()

        const originalMessage = new ClientSetup(setupParams)
        const messageBytes = originalMessage.serialize().toUint8Array()
        mockBidirectionalStream = createMockBidirectionalStream([messageBytes])
        controlStream = ControlStream.new(mockBidirectionalStream)

        await controlStream.send(originalMessage)
        const reader = controlStream.stream.getReader()
        const { value: receivedMessage } = await reader.read()
        expect(receivedMessage).toBeInstanceOf(ClientSetup)
        expect(receivedMessage).toEqual(originalMessage)
        reader.releaseLock()
      })
      it('should handle excess bytes successful roundtrip then timeout', async () => {
        const setupParams = new SetupParameters().addPath('/excess/test').build()

        const originalMessage = new ClientSetup(setupParams)
        const messageBytes = originalMessage.serialize().toUint8Array()
        const excessBytes = new Uint8Array([0xff, 0x13, 0x25])

        const combinedBytes = new Uint8Array(messageBytes.length + excessBytes.length)
        combinedBytes.set(messageBytes, 0)
        combinedBytes.set(excessBytes, messageBytes.length)

        mockBidirectionalStream = createMockBidirectionalStream([combinedBytes])
        controlStream = ControlStream.new(mockBidirectionalStream, 250)
        const reader = controlStream.stream.getReader()
        const { value: receivedMessage } = await reader.read()
        expect(receivedMessage).toEqual(originalMessage)
        await expect(reader.read()).rejects.toThrow(TimeoutError)
        reader.releaseLock()
      })
      it('should timeout on partial message', async () => {
        const setupParams = new SetupParameters().addPath('/partial/test').addMaxRequestId(42n).build()
        const originalMessage = new ClientSetup(setupParams)
        const completeMessageBytes = originalMessage.serialize().toUint8Array()

        // Send only partial message (first 10 bytes)
        const partialBytes = completeMessageBytes.slice(0, Math.min(10, completeMessageBytes.length))

        mockBidirectionalStream = createMockBidirectionalStream([partialBytes])
        controlStream = ControlStream.new(mockBidirectionalStream, 250)
        const reader = controlStream.stream.getReader()
        await expect(reader.read()).rejects.toThrow(TerminationError)
        reader.releaseLock()
      })
    })
  })
}
