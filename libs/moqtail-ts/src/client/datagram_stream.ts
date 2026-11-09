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

import type { MoqtObject } from '@/model/data'
import type { Emitter } from '@/util/emitter'
import { logger } from '@/util/logger'
import type { ClientEvents } from './types'

/**
 * Sends MoqtObjects as WebTransport datagrams.
 * Parallel to SendStream but for datagram-based delivery.
 *
 * @example
 * ```ts
 * const sender = await SendDatagramStream.new(client.webTransport.datagrams.writable, trackAlias);
 * await sender.write(moqtObject);
 * ```
 */
export class SendDatagramStream {
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>
  readonly #trackAlias: bigint
  readonly #events: Emitter<ClientEvents> | undefined

  constructor(writer: WritableStreamDefaultWriter<Uint8Array>, trackAlias: bigint, events?: Emitter<ClientEvents>) {
    this.#writer = writer
    this.#trackAlias = trackAlias
    this.#events = events
  }

  /**
   * Create a new datagram sender for a specific track.
   *
   * @param writeStream - WebTransport datagram writable stream
   * @param trackAlias - Track alias for this datagram stream
   * @param events - Receives `datagramsent` for each datagram written
   * @returns SendDatagramStream instance
   */
  static async new(
    writeStream: WritableStream<Uint8Array>,
    trackAlias: bigint,
    events?: Emitter<ClientEvents>,
  ): Promise<SendDatagramStream> {
    const writer = writeStream.getWriter()
    return new SendDatagramStream(writer, trackAlias, events)
  }

  /**
   * Create a datagram sender using an existing shared writer.
   * Use this when multiple senders need to share a single writer.
   *
   * @param writer - Existing WritableStreamDefaultWriter
   * @param trackAlias - Track alias for this datagram stream
   * @param events - Receives `datagramsent` for each datagram written
   * @returns SendDatagramStream instance
   */
  static fromWriter(
    writer: WritableStreamDefaultWriter<Uint8Array>,
    trackAlias: bigint,
    events?: Emitter<ClientEvents>,
  ): SendDatagramStream {
    return new SendDatagramStream(writer, trackAlias, events)
  }

  /**
   * Write a MoqtObject as a datagram.
   * Converts to Datagram automatically based on object state.
   *
   * @param object - MoqtObject to send (must have Datagram forwarding preference)
   */
  async write(object: MoqtObject): Promise<void> {
    const datagram = object.tryIntoDatagram(this.#trackAlias)
    const serialized = datagram.serialize().toUint8Array()
    logger.log(
      'datagram_stream',
      `Writing datagram: trackAlias=${this.#trackAlias}, group=${object.location?.group}, obj=${object.location?.object}, size=${serialized.byteLength}`,
    )
    this.#events?.emit('datagramsent', datagram)

    try {
      await this.#writer.write(serialized)
      logger.log('datagram_stream', `Successfully wrote ${serialized.byteLength} bytes to WebTransport datagram`)
    } catch (err) {
      logger.error('datagram_stream', `ERROR writing datagram:`, err)
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
