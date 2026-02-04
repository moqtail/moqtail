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

import { DatagramObject, DatagramStatus } from '../model/data'
import { MoqtObject, FullTrackName } from '../model/data'
import { ProtocolViolationError } from '../model/error/error'

/**
 * Sends MoqtObjects as WebTransport datagrams.
 * Parallel to SendStream but for datagram-based delivery.
 *
 * @example
 * ```ts
 * const sender = new SendDatagramStream(client.webTransport.datagrams.writable, trackAlias);
 * await sender.write(moqtObject);
 * ```
 */
export class SendDatagramStream {
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>
  readonly #trackAlias: bigint
  readonly onDataSent?: (data: DatagramObject | DatagramStatus) => void

  constructor(
    writer: WritableStreamDefaultWriter<Uint8Array>,
    trackAlias: bigint,
    onDataSent?: (data: DatagramObject | DatagramStatus) => void,
  ) {
    this.#writer = writer
    this.#trackAlias = trackAlias
    if (onDataSent) this.onDataSent = onDataSent
  }

  /**
   * Create a new datagram sender for a specific track.
   *
   * @param writeStream - WebTransport datagram writable stream
   * @param trackAlias - Track alias for this datagram stream
   * @param onDataSent - Optional callback fired when datagram is sent
   * @returns SendDatagramStream instance
   */
  static async new(
    writeStream: WritableStream<Uint8Array>,
    trackAlias: bigint,
    onDataSent?: (data: DatagramObject | DatagramStatus) => void,
  ): Promise<SendDatagramStream> {
    const writer = writeStream.getWriter()
    return new SendDatagramStream(writer, trackAlias, onDataSent)
  }

  /**
   * Create a datagram sender using an existing shared writer.
   * Use this when multiple senders need to share a single writer.
   *
   * @param writer - Existing WritableStreamDefaultWriter
   * @param trackAlias - Track alias for this datagram stream
   * @param onDataSent - Optional callback fired when datagram is sent
   * @returns SendDatagramStream instance
   */
  static fromWriter(
    writer: WritableStreamDefaultWriter<Uint8Array>,
    trackAlias: bigint,
    onDataSent?: (data: DatagramObject | DatagramStatus) => void,
  ): SendDatagramStream {
    return new SendDatagramStream(writer, trackAlias, onDataSent)
  }

  /**
   * Write a MoqtObject as a datagram.
   * Converts to DatagramObject or DatagramStatus automatically.
   *
   * @param object - MoqtObject to send (must have Datagram forwarding preference)
   * @throws ProtocolViolationError if object is not datagram-compatible
   */
  async write(object: MoqtObject): Promise<void> {
    let serialized: Uint8Array

    if (object.hasStatus()) {
      const datagramStatus = object.tryIntoDatagramStatus(this.#trackAlias)
      serialized = datagramStatus.serialize().toUint8Array()
      console.log(
        `[SendDatagramStream] Writing status datagram: trackAlias=${this.#trackAlias}, size=${serialized.byteLength}`,
      )
      if (this.onDataSent) this.onDataSent(datagramStatus)
    } else if (object.hasPayload()) {
      const datagramObject = object.tryIntoDatagramObject(this.#trackAlias)
      serialized = datagramObject.serialize().toUint8Array()
      console.log(
        `[SendDatagramStream] Writing object datagram: trackAlias=${this.#trackAlias}, group=${object.location?.group}, obj=${object.location?.object}, payloadSize=${object.payload?.byteLength}, serializedSize=${serialized.byteLength}`,
      )
      if (this.onDataSent) this.onDataSent(datagramObject)
    } else {
      throw new ProtocolViolationError('SendDatagramStream.write', 'MoqtObject must have either payload or status')
    }

    try {
      await this.#writer.write(serialized)
      console.log(`[SendDatagramStream] Successfully wrote ${serialized.byteLength} bytes to WebTransport datagram`)
    } catch (err) {
      console.error(`[SendDatagramStream] ERROR writing datagram:`, err)
      throw err
    }
  }

  /**
   * Close the datagram writer.
   */
  async close(): Promise<void> {
    if (this.#writer) {
      await this.#writer.close()
    }
  }

  /**
   * Release the writer lock without closing.
   */
  releaseLock(): void {
    this.#writer.releaseLock()
  }
}

/**
 * Receives and parses WebTransport datagrams as MoqtObjects.
 * Parallel to RecvStream but for datagram-based delivery.
 *
 * Automatically handles:
 * - DatagramObject parsing (objects with payloads)
 * - DatagramStatus parsing (status-only objects)
 * - Track alias to full track name resolution
 *
 * @example
 * ```ts
 * const receiver = new RecvDatagramStream(
 *   client.webTransport.datagrams.readable,
 *   (trackAlias) => client.requestIdMap.getNameByTrackAlias(trackAlias)
 * );
 *
 * for await (const object of receiver.stream) {
 *   console.log('Received:', object);
 * }
 * ```
 */
export class RecvDatagramStream {
  readonly stream: ReadableStream<MoqtObject>
  readonly #reader: ReadableStreamDefaultReader<Uint8Array>
  readonly #trackAliasResolver: (trackAlias: bigint) => FullTrackName
  readonly onDataReceived?: (data: DatagramObject | DatagramStatus) => void

  private constructor(
    reader: ReadableStreamDefaultReader<Uint8Array>,
    trackAliasResolver: (trackAlias: bigint) => FullTrackName,
    onDataReceived?: (data: DatagramObject | DatagramStatus) => void,
  ) {
    this.#reader = reader
    this.#trackAliasResolver = trackAliasResolver
    if (onDataReceived) this.onDataReceived = onDataReceived

    this.stream = new ReadableStream<MoqtObject>({
      start: (controller) => this.#ingestLoop(controller),
      cancel: () => this.#reader.cancel(),
    })
  }

  /**
   * Create a new datagram receiver.
   *
   * @param readStream - WebTransport datagram readable stream
   * @param trackAliasResolver - Function to resolve track alias to full track name
   * @param onDataReceived - Optional callback fired when datagram is received
   * @returns RecvDatagramStream instance
   */
  static async new(
    readStream: ReadableStream<Uint8Array>,
    trackAliasResolver: (trackAlias: bigint) => FullTrackName,
    onDataReceived?: (data: DatagramObject | DatagramStatus) => void,
  ): Promise<RecvDatagramStream> {
    const reader = readStream.getReader()
    return new RecvDatagramStream(reader, trackAliasResolver, onDataReceived)
  }

  async #ingestLoop(controller: ReadableStreamDefaultController<MoqtObject>) {
    try {
      while (true) {
        const { done, value: datagramBytes } = await this.#reader.read()

        if (done) {
          controller.close()
          break
        }

        if (!datagramBytes || datagramBytes.length === 0) {
          continue
        }

        try {
          // Try to parse as datagram (peek at first byte to determine type)
          const firstByte = datagramBytes[0]
          const isStatus = firstByte === 0x02 || firstByte === 0x03

          let moqtObject: MoqtObject

          if (isStatus) {
            // DatagramStatus (0x02 or 0x03)
            const datagramStatus = DatagramStatus.deserialize(
              new (await import('../model/common/byte_buffer')).FrozenByteBuffer(datagramBytes),
            )
            if (this.onDataReceived) this.onDataReceived(datagramStatus)

            const fullTrackName = this.#trackAliasResolver(datagramStatus.trackAlias)
            moqtObject = MoqtObject.fromDatagramStatus(datagramStatus, fullTrackName)
          } else {
            // DatagramObject (0x00 or 0x01)
            const datagramObject = DatagramObject.deserialize(
              new (await import('../model/common/byte_buffer')).FrozenByteBuffer(datagramBytes),
            )
            if (this.onDataReceived) this.onDataReceived(datagramObject)

            const fullTrackName = this.#trackAliasResolver(datagramObject.trackAlias)
            moqtObject = MoqtObject.fromDatagramObject(datagramObject, fullTrackName)
          }

          controller.enqueue(moqtObject)
        } catch (error) {
          // Log but don't break the stream - individual datagrams may be corrupt
          console.warn('Failed to parse datagram:', error)
          continue
        }
      }
    } catch (error) {
      controller.error(error)
    }
  }

  /**
   * Cancel the datagram reader.
   */
  async cancel(): Promise<void> {
    await this.#reader.cancel()
  }

  /**
   * Release the reader lock without canceling.
   */
  releaseLock(): void {
    this.#reader.releaseLock()
  }
}
