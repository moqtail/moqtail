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

import { MOQtailClient } from './client'
import { SendDatagramStream } from './datagram_stream'
import { DatagramObject, DatagramStatus, MoqtObject, FullTrackName } from '../model/data'
import { InternalError, MOQtailError } from '../model'

/**
 * Options for datagram-enabled MOQtail client.
 * Extends base client options with datagram-specific callbacks.
 */
export type DatagramCallbacks = {
  /**
   * Invoked for each decoded datagram object/status arriving.
   * Use to process or display incoming datagram-based media/data.
   */
  onDatagramReceived?: (data: DatagramObject | DatagramStatus) => void

  /**
   * Invoked after enqueuing each outbound datagram object/status.
   * Use for monitoring or analytics.
   */
  onDatagramSent?: (data: DatagramObject | DatagramStatus) => void
}

/**
 * Datagram-enabled MOQtail client wrapper.
 * Provides datagram send/receive capabilities on top of standard MOQtailClient.
 * 
 * @example Basic setup
 * ```ts
 * const client = await MOQtailClient.new({ url, supportedVersions });
 * const datagramClient = new DatagramClient(client, {
 *   onDatagramReceived: (data) => console.log('Received:', data),
 *   onDatagramSent: (data) => console.log('Sent:', data)
 * });
 * await datagramClient.start();
 * ```
 * 
 * @example Sending datagrams
 * ```ts
 * const sender = datagramClient.createDatagramSender(trackAlias);
 * await sender.write(moqtObject);
 * ```
 * 
 * @example Subscribing to a specific track
 * ```ts
 * datagramClient.subscribeToTrack(trackAlias, (obj) => {
 *   console.log('Received object for track:', obj);
 * });
 * ```
 */
export class DatagramClient {
  readonly client: MOQtailClient
  readonly callbacks: DatagramCallbacks | undefined
  
  #datagramWriter: WritableStreamDefaultWriter<Uint8Array> | undefined
  #datagramReader: ReadableStreamDefaultReader<Uint8Array> | undefined
  #isReceivingDatagrams = false
  #receivedObjectController?: ReadableStreamDefaultController<MoqtObject>

  /**
   * Per-track handlers for received datagrams.
   * Key is track alias (as string for Map compatibility), value is handler function.
   */
  #trackHandlers: Map<string, (obj: MoqtObject) => void> = new Map()

  /**
   * Stream of all received MoqtObjects from datagrams across all tracks.
   * Consumer should filter by fullTrackName as needed.
   * 
   * WARNING: Only one reader should be active. For multiple subscribers,
   * use subscribeToTrack() instead.
   */
  readonly receivedObjects: ReadableStream<MoqtObject>

  constructor(client: MOQtailClient, callbacks?: DatagramCallbacks) {
    this.client = client
    this.callbacks = callbacks

    // Create a stream for received datagram objects
    this.receivedObjects = new ReadableStream<MoqtObject>({
      start: (controller) => {
        this.#receivedObjectController = controller
      },
      cancel: () => this.stop(),
    })
  }

  /**
   * Start receiving datagrams from the WebTransport connection.
   * Must be called before datagrams can be received.
   * 
   * @throws MOQtailError if client is destroyed or datagrams already started
   */
  async start(): Promise<void> {
    console.log('[DatagramClient] Starting...')
    if (this.#isReceivingDatagrams) {
      console.warn('[DatagramClient] Already started')
      throw new MOQtailError('Datagram receiving already started')
    }

    console.log('[DatagramClient] Getting datagram reader and writer from WebTransport')
    this.#datagramReader = this.client.webTransport.datagrams.readable.getReader()
    this.#datagramWriter = this.client.webTransport.datagrams.writable.getWriter()
    this.#isReceivingDatagrams = true
    console.log('[DatagramClient] Started successfully, beginning datagram reception')
    
    // Start background datagram reception
    this.#acceptIncomingDatagrams()
  }

  /**
   * Subscribe to receive datagrams for a specific track.
   * Multiple tracks can have separate handlers that run concurrently.
   * 
   * @param trackAlias - Track alias to subscribe to
   * @param handler - Function called for each received MoqtObject on this track
   * @returns Unsubscribe function to remove the handler
   * 
   * @example
   * ```ts
   * const unsubscribe = datagramClient.subscribeToTrack(trackAlias, (obj) => {
   *   console.log('Received:', obj.payload);
   * });
   * // Later: unsubscribe();
   * ```
   */
  subscribeToTrack(trackAlias: bigint, handler: (obj: MoqtObject) => void): () => void {
    const key = trackAlias.toString()
    console.log(`[DatagramClient] Registering handler for trackAlias=${trackAlias}, key="${key}"`)
    console.log(`[DatagramClient] Current registered handlers before: ${Array.from(this.#trackHandlers.keys()).join(', ') || '(none)'}`)
    this.#trackHandlers.set(key, handler)
    console.log(`[DatagramClient] Current registered handlers after: ${Array.from(this.#trackHandlers.keys()).join(', ')}`)
    
    return () => {
      console.log(`[DatagramClient] Unregistering handler for trackAlias=${trackAlias}`)
      this.#trackHandlers.delete(key)
    }
  }

  /**
   * Unsubscribe from a specific track.
   * 
   * @param trackAlias - Track alias to unsubscribe from
   */
  unsubscribeFromTrack(trackAlias: bigint): void {
    const key = trackAlias.toString()
    this.#trackHandlers.delete(key)
  }

  /**
   * Stop receiving datagrams and release resources.
   * Idempotent - safe to call multiple times.
   */
  async stop(): Promise<void> {
    if (!this.#isReceivingDatagrams) return
    
    this.#isReceivingDatagrams = false
    this.#trackHandlers.clear()

    if (this.#datagramReader) {
      await this.#datagramReader.cancel().catch(() => {})
      this.#datagramReader.releaseLock()
      this.#datagramReader = undefined
    }

    if (this.#datagramWriter) {
      await this.#datagramWriter.close().catch(() => {})
      this.#datagramWriter = undefined
    }

    if (this.#receivedObjectController) {
      this.#receivedObjectController.close()
    }
  }

  /**
   * Create a datagram sender for a specific track.
   * 
   * @param trackAlias - Track alias for outgoing datagrams
   * @returns SendDatagramStream for writing MoqtObjects as datagrams
   * @throws MOQtailError if datagram writer not initialized (call start() first)
   * 
   * @example
   * ```ts
   * const sender = datagramClient.createDatagramSender(trackAlias);
   * await sender.write(moqtObject);
   * // Note: Do NOT call releaseLock() - the writer is shared
   * ```
   */
  createDatagramSender(trackAlias: bigint): SendDatagramStream {
    console.log(`[DatagramClient] Creating datagram sender for trackAlias=${trackAlias}`)
    if (!this.#datagramWriter) {
      console.error(`[DatagramClient] ERROR: Cannot create sender - datagram writer not initialized`)
      throw new MOQtailError('DatagramClient not started. Call start() first.')
    }

    console.log(`[DatagramClient] Datagram sender created successfully for trackAlias=${trackAlias}`)
    // Use the shared writer - all senders share the same underlying writer
    return SendDatagramStream.fromWriter(this.#datagramWriter, trackAlias, this.callbacks?.onDatagramSent)
  }

  /**
   * Send a single MoqtObject as a datagram.
   * Convenience method that creates a temporary sender.
   * 
   * @param trackAlias - Track alias for this object
   * @param object - MoqtObject to send
   * @throws MOQtailError if datagram writer not initialized
   * 
   * @example
   * ```ts
   * await datagramClient.sendDatagram(trackAlias, moqtObject);
   * ```
   */
  async sendDatagram(trackAlias: bigint, object: MoqtObject): Promise<void> {
    if (!this.#datagramWriter) {
      throw new MOQtailError('DatagramClient not started. Call start() first.')
    }

    let serialized: Uint8Array

    if (object.hasStatus()) {
      const datagramStatus = object.tryIntoDatagramStatus(trackAlias)
      serialized = datagramStatus.serialize().toUint8Array()
      if (this.callbacks?.onDatagramSent) this.callbacks.onDatagramSent(datagramStatus)
    } else if (object.hasPayload()) {
      const datagramObject = object.tryIntoDatagramObject(trackAlias)
      serialized = datagramObject.serialize().toUint8Array()
      if (this.callbacks?.onDatagramSent) this.callbacks.onDatagramSent(datagramObject)
    } else {
      throw new InternalError('DatagramClient.sendDatagram', 'MoqtObject must have payload or status')
    }

    await this.#datagramWriter.write(serialized)
  }

  /**
   * Background loop that receives and parses incoming datagrams.
   * Runs until stop() is called or an error occurs.
   */
  async #acceptIncomingDatagrams(): Promise<void> {
    console.log('[DatagramClient] Starting datagram reception loop')
    try {
      while (this.#isReceivingDatagrams && this.#datagramReader) {
        const { done, value: datagramBytes } = await this.#datagramReader.read()

        if (done) {
          console.log('[DatagramClient] Datagram reader done, stopping reception')
          this.#isReceivingDatagrams = false
          if (this.#receivedObjectController) {
            this.#receivedObjectController.close()
          }
          break
        }

        if (!datagramBytes || datagramBytes.length === 0) {
          console.log('[DatagramClient] Received empty datagram, skipping')
          continue
        }

        const firstByte = datagramBytes[0]!
        console.log(`[DatagramClient] Received datagram: ${datagramBytes.length} bytes, firstByte=0x${firstByte.toString(16)}`)

        try {
          // Parse datagram (peek at first byte to determine type)
          const isStatus = firstByte === 0x02 || firstByte === 0x03

          let moqtObject: MoqtObject
          let trackAlias: bigint

          if (isStatus) {
            // DatagramStatus (0x02 or 0x03)
            console.log('[DatagramClient] Parsing as DatagramStatus')
            const { FrozenByteBuffer } = await import('../model/common/byte_buffer')
            const datagramStatus = DatagramStatus.deserialize(new FrozenByteBuffer(datagramBytes))
            trackAlias = datagramStatus.trackAlias
            console.log(`[DatagramClient] Parsed status: trackAlias=${trackAlias}`)
            
            if (this.callbacks?.onDatagramReceived) {
              this.callbacks.onDatagramReceived(datagramStatus)
            }

            // Resolve track alias to full track name
            const fullTrackName = this.#resolveTrackAlias(trackAlias)
            moqtObject = MoqtObject.fromDatagramStatus(datagramStatus, fullTrackName)
          } else {
            // DatagramObject (0x00 or 0x01)
            console.log('[DatagramClient] Parsing as DatagramObject')
            const { FrozenByteBuffer } = await import('../model/common/byte_buffer')
            const datagramObject = DatagramObject.deserialize(new FrozenByteBuffer(datagramBytes))
            trackAlias = datagramObject.trackAlias
            console.log(`[DatagramClient] Parsed object: trackAlias=${trackAlias}, group=${datagramObject.groupId}, object=${datagramObject.objectId}, payloadSize=${datagramObject.payload?.byteLength}`)
            
            if (this.callbacks?.onDatagramReceived) {
              this.callbacks.onDatagramReceived(datagramObject)
            }

            // Resolve track alias to full track name
            const fullTrackName = this.#resolveTrackAlias(trackAlias)
            moqtObject = MoqtObject.fromDatagramObject(datagramObject, fullTrackName)
          }

          // Dispatch to track-specific handler if registered
          const trackKey = trackAlias.toString()
          const handler = this.#trackHandlers.get(trackKey)
          console.log(`[DatagramClient] Looking for handler for trackAlias=${trackAlias}, key="${trackKey}", found=${!!handler}`)
          if (handler) {
            try {
              console.log(`[DatagramClient] Dispatching to track handler`)
              handler(moqtObject)
            } catch (handlerError) {
              console.warn('[DatagramClient] Track handler error:', handlerError)
            }
          } else {
            console.log(`[DatagramClient] No handler registered. Registered handlers: ${Array.from(this.#trackHandlers.keys()).join(', ')}`)
          }

          // Also enqueue to the general stream (for backwards compatibility)
          if (this.#receivedObjectController) {
            this.#receivedObjectController.enqueue(moqtObject)
          }
        } catch (error) {
          // Log but don't break - individual datagrams may be corrupt/unknown
          console.warn('[DatagramClient] Failed to parse datagram:', error)
          continue
        }
      }
    } catch (error) {
      console.error('[DatagramClient] Datagram reception error (loop ended):', error)
      if (this.#receivedObjectController) {
        this.#receivedObjectController.error(error)
      }
      this.#isReceivingDatagrams = false
    }
  }

  /**
   * Resolve track alias to full track name using client's request ID map.
   * Falls back to a placeholder if not found.
   */
  #resolveTrackAlias(trackAlias: bigint): FullTrackName {
    try {
      // Try to get from subscription alias map
      const requestId = this.client.subscriptionAliasMap.get(trackAlias)
      if (requestId !== undefined) {
        return this.client.requestIdMap.getNameByRequestId(requestId)
      }

      // Fallback: return placeholder
      return FullTrackName.tryNew('unknown', `track-${trackAlias}`)
    } catch {
      return FullTrackName.tryNew('unknown', `track-${trackAlias}`)
    }
  }
}
