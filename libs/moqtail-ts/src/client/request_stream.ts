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
import { NotEnoughBytesError, TerminationError } from '../model/error/error'
import { TerminationCode } from '../model/error/constant'
import { logger } from '../util/logger'

/**
 * Wraps a WebTransport bidirectional stream for request-style MOQT streams
 * such as SUBSCRIBE_NAMESPACE.
 */
export class RequestStream {
  readonly stream: ReadableStream<ControlMessage>
  readonly #reader: ReadableStreamDefaultReader<Uint8Array>
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>
  #receiveBuffer: ByteBuffer

  constructor(biStream: WebTransportBidirectionalStream) {
    this.#receiveBuffer = new ByteBuffer()
    this.#reader = biStream.readable.getReader()
    this.#writer = biStream.writable.getWriter()
    this.stream = new ReadableStream<ControlMessage>({
      start: (controller) => this.#ingestLoop(controller),
      cancel: () => this.close(),
    })
    logger.debug('request_stream', 'opened')
  }

  async send(message: ControlMessage): Promise<void> {
    try {
      const serialized = ControlMessage.serialize(message)
      await this.#writer.ready
      await this.#writer.write(serialized.toUint8Array())
      logger.debug('request_stream', `sent ${message.constructor.name}`)
    } catch (error: any) {
      await this.close()
      const msg = error instanceof Error ? error.message : String(error)
      logger.error('request_stream', `send failed: ${msg}`)
      throw new TerminationError(`RequestStream.send: Failed to write message: ${msg}`, TerminationCode.INTERNAL_ERROR)
    }
  }

  async close(): Promise<void> {
    logger.debug('request_stream', 'closed')
    await Promise.allSettled([this.#writer.close().catch(() => {}), this.#reader.cancel().catch(() => {})])
  }

  async #ingestLoop(controller: ReadableStreamDefaultController<ControlMessage>) {
    try {
      while (true) {
        if (this.#receiveBuffer.length === 0) {
          const readResult = await this.#reader.read()
          this.#handleReadResult(readResult)
          if (readResult.done) break
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
            const msg = ControlMessage.deserialize(new FrozenByteBuffer(messageBytes))
            controller.enqueue(msg)
            continue
          }
          const readResult = await this.#reader.read()
          this.#handleReadResult(readResult)
          if (readResult.done) break
        } catch (error: any) {
          if (error instanceof NotEnoughBytesError) {
            const readResult = await this.#reader.read()
            this.#handleReadResult(readResult)
            if (readResult.done) break
          } else {
            logger.error('request_stream', `deserialization error: ${error.message}`)
            controller.error(
              new TerminationError(
                `RequestStream: Deserialization error: ${error.message}`,
                TerminationCode.PROTOCOL_VIOLATION,
              ),
            )
            await this.close()
            break
          }
        }
      }
    } catch (error) {
      logger.error('request_stream', 'ingest loop error', error)
      controller.error(error)
      await this.close()
    } finally {
      controller.close()
      await this.close()
    }
  }

  #handleReadResult(readResult: ReadableStreamReadResult<Uint8Array>): void {
    if (readResult.done) return
    if (readResult.value) {
      this.#receiveBuffer.putBytes(readResult.value)
    }
  }
}
