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

import { ControlStream } from './control_stream'
import { SendDatagramStream } from './datagram_stream'
import {
  PublishNamespace,
  PublishNamespaceDone,
  PublishNamespaceError,
  PublishNamespaceOk,
  ClientSetup,
  ControlMessage,
  Fetch,
  FetchCancel,
  FetchError,
  FetchType,
  FilterType,
  GoAway,
  GroupOrder,
  ServerSetup,
  Subscribe,
  SubscribeNamespace,
  SubscribeError,
  SubscribeUpdate,
  Unsubscribe,
  UnsubscribeNamespace,
} from '../model/control'
import {
  DatagramObject,
  DatagramStatus,
  FetchHeader,
  FetchObject,
  FullTrackName,
  MoqtObject,
  SubgroupHeader,
  SubgroupHeaderType,
  SubgroupObject,
  RequestIdMap,
} from '../model/data'
import { FrozenByteBuffer } from '../model/common/byte_buffer'
import { RecvStream } from './data_stream'
import {
  InternalError,
  MOQtailError,
  ProtocolViolationError,
  SetupParameters,
  Tuple,
  VersionSpecificParameters,
} from '../model'
import { PublishNamespaceCancel } from '../model/control/publish_namespace_cancel'
import { Track } from './track/track'
import { PublishNamespaceRequest } from './request/publish_namespace'
import { FetchRequest } from './request/fetch'
import { SubscribeRequest } from './request/subscribe'
import { getHandlerForControlMessage } from './handler/handler'
import { SubscribePublication } from './publication/subscribe'
import { FetchPublication } from './publication/fetch'

/**
 * Union type of all possible MOQtail requests
 */
export type MOQtailRequest = PublishNamespaceRequest | FetchRequest | SubscribeRequest

/**
 * Callbacks related to datagram events
 */
export type DatagramCallbacks = {
  /**
   * Invoked for each decoded datagram object/status arriving
   * Use to process or display incoming datagram-based media/data
   */
  onDatagramReceived?: (data: DatagramObject | DatagramStatus) => void

  /**
   * Invoked after enqueuing each outbound datagram object/status
   * Use for monitoring or analytics
   */
  onDatagramSent?: (data: DatagramObject | DatagramStatus) => void
}

/**
 * Extended options for the MOQtail client
 *
 * @example Minimal
 * ```ts
 * const client = await MOQtailClient.new({
 *   url: 'https://relay.example.com/moq',
 *   supportedVersions: [0xff00000b]
 * })
 * ```
 *
 * @example With datagram support
 * ```ts
 * const client = await MOQtailClient.new({
 *   url: relayUrl,
 *   supportedVersions: [0xff00000b],
 *   enableDatagrams: true,
 *   callbacks: {
 *     onDatagramReceived: (data) => console.log('Datagram received:', data),
 *     onDatagramSent: (data) => console.log('Datagram sent:', data),
 *   }
 * })
 * ```
 */
export type MOQtailClientOptions = {
  /** Relay / server endpoint for the underlying WebTransport session. */
  url: string | URL
  /** Ordered preference list of MOQT protocol version numbers (e.g. `0xff00000b`). */
  supportedVersions: number[]
  /** SetupParameters customizations; if omitted a default instance is built. */
  setupParameters?: SetupParameters
  /** Passed directly to the browser's WebTransport constructor. */
  transportOptions?: WebTransportOptions
  /** Per *data* uni-stream idle timeout in milliseconds. */
  dataStreamTimeoutMs?: number
  /** Control stream read timeout in milliseconds. */
  controlStreamTimeoutMs?: number
  /**
   * Enable datagram support. When true, datagrams will be automatically started
   * after connection is established. Default: false
   */
  enableDatagrams?: boolean
  /** Callbacks for observability and logging purposes. */
  callbacks?: {
    /** Called after a control message is successfully written to the ControlStream. */
    onMessageSent?: (msg: ControlMessage) => void
    /** Called for each incoming control message before protocol handling. */
    onMessageReceived?: (msg: ControlMessage) => void
    /** Fired once when the session ends (normal or error). */
    onSessionTerminated?: (reason?: unknown) => void
    /** Invoked for each decoded datagram object/status arriving. */
    onDatagramReceived?: (data: DatagramObject | DatagramStatus) => void
    /** Invoked after enqueuing each outbound datagram object/status. */
    onDatagramSent?: (data: DatagramObject | DatagramStatus) => void
  }
}

/**
 * Parameters for subscribing to a track's live objects
 */
export type SubscribeOptions = {
  fullTrackName: FullTrackName
  priority: number
  groupOrder: GroupOrder
  forward: boolean
  filterType: FilterType
  parameters?: VersionSpecificParameters
  startLocation?: import('../model').Location
  endGroup?: bigint | number
}

/**
 * Narrowing update constraints applied to an existing SUBSCRIBE
 */
export type SubscribeUpdateOptions = {
  subscriptionRequestId: bigint
  startLocation: import('../model').Location
  endGroup: bigint
  priority: number
  forward: boolean
  parameters?: VersionSpecificParameters
}

/**
 * Options for performing a FETCH operation for historical or relative object ranges
 */
export type FetchOptions = {
  priority: number
  groupOrder: GroupOrder
  typeAndProps:
    | {
        type: FetchType.StandAlone
        props: {
          fullTrackName: FullTrackName
          startLocation: import('../model').Location
          endLocation: import('../model').Location
        }
      }
    | {
        type: FetchType.Relative
        props: { joiningRequestId: bigint; joiningStart: bigint }
      }
    | {
        type: FetchType.Absolute
        props: { joiningRequestId: bigint; joiningStart: bigint }
      }
  parameters?: VersionSpecificParameters
}
/**
 * @public
 * MOQtailClient is the main entry point for interacting with a MOQT server over WebTransport.
 *
 * It manages the underlying WebTransport session, control stream, data streams,
 * track subscriptions, publications, and datagram support.
 *
 * Clients can register tracks for publishing, subscribe to tracks for receiving data,
 * perform fetch operations, and send/receive datagrams.
 *
 * Extensive callback hooks are provided for observability and custom handling of events.
 *
 * Example usage:
 * ```ts
 * const client = await MOQtailClient.new({
 *   url: 'https://relay.example.com/moq',
 *  supportedVersions: [0xff00000b],
 *  enableDatagrams: true,
 *  callbacks: {
 *    onDatagramReceived: (data) => console.log('Datagram received:', data),
 *   onDatagramSent: (data) => console.log('Datagram sent:', data),
 * }
 * })
 * ```
 *
 */
export class MOQtailClient {
  /**
   * Namespace prefixes (tuples) the peer has requested announce notifications for via SUBSCRIBE_NAMESPACE
   */
  readonly peerSubscribeNamespace = new Set<Tuple>()

  /**
   * Namespace prefixes this client has subscribed to (issued SUBSCRIBE_NAMESPACE)
   */
  readonly subscribedAnnounces = new Set<Tuple>()

  /**
   * Track namespaces this client has successfully announced (received ANNOUNCE_OK)
   */
  readonly announcedNamespaces = new Set<Tuple>()

  /**
   * Locally registered track definitions keyed by full track name string
   */
  readonly trackSources: Map<string, Track> = new Map()

  /**
   * All in‑flight request objects keyed by requestId
   */
  readonly requests: Map<bigint, MOQtailRequest> = new Map()

  /**
   * Active publications keyed by requestId
   */
  readonly publications: Map<bigint, SubscribePublication | FetchPublication> = new Map()

  /**
   * Active SUBSCRIBE request wrappers keyed by track alias
   */
  readonly subscriptions: Map<bigint, SubscribeRequest> = new Map()

  /**
   * Bidirectional track alias-subscription requestId mapping
   */
  readonly subscriptionAliasMap: Map<bigint, bigint> = new Map()

  /**
   * Bidirectional requestId-full track name mapping
   */
  readonly requestIdMap: RequestIdMap = new RequestIdMap()

  /** Underlying WebTransport session. */
  webTransport!: WebTransport

  /** Validated ServerSetup message captured during handshake. */
  #serverSetup!: ServerSetup

  /** Outgoing / incoming control message bidirectional stream wrapper. */
  controlStream!: ControlStream

  /** Timeout (ms) applied to reading incoming data streams. */
  dataStreamTimeoutMs?: number

  /** Timeout (ms) for control stream read operations. */
  controlStreamTimeoutMs?: number

  /** Optional highest request id allowed. */
  maxRequestId?: bigint

  /** Flag indicating the client has been disconnected/destroyed. */
  #isDestroyed = false

  /** Internal monotonically increasing client-assigned request id counter. */
  #dontUseRequestId: bigint = 0n

  // Original MOQtailClient Event Handlers

  /** Fired when a PUBLISH_NAMESPACE control message is processed. */
  onNamespacePublished?: (msg: PublishNamespace) => void

  /** Fired when a PUBLISH_NAMESPACE_DONE control message is processed. */
  onNamespaceDone?: (msg: PublishNamespaceDone) => void

  /** Fired on GOAWAY reception signaling graceful session wind-down. */
  onGoaway?: (msg: GoAway) => void

  /** Fired if the underlying WebTransport session fails. */
  onWebTransportFail?: () => void

  /** Fired exactly once when the client transitions to terminated. */
  onSessionTerminated?: (reason?: unknown) => void

  /** Invoked after each outbound control message is sent. */
  onMessageSent?: (msg: ControlMessage) => void

  /** Invoked upon receiving each inbound control message. */
  onMessageReceived?: (msg: ControlMessage) => void

  /** Invoked for each decoded data object/header arriving on a uni stream. */
  onDataReceived?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void

  /** Invoked after enqueuing each outbound data object/header. */
  onDataSent?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void

  /** General-purpose error callback. */
  onError?: (er: unknown) => void

  // Datagram-specific Properties

  /** Invoked for each decoded datagram object/status arriving. */
  onDatagramReceived?: (data: DatagramObject | DatagramStatus) => void

  /** Invoked after enqueuing each outbound datagram object/status. */
  onDatagramSent?: (data: DatagramObject | DatagramStatus) => void

  /** Datagram writer for sending datagrams. */
  #datagramWriter: WritableStreamDefaultWriter<Uint8Array> | undefined

  /** Datagram reader for receiving datagrams. */
  #datagramReader: ReadableStreamDefaultReader<Uint8Array> | undefined

  /** Flag indicating if datagram reception loop is active. */
  #isReceivingDatagrams = false

  /** Controller for the received objects stream. */
  #receivedDatagramObjectController?: ReadableStreamDefaultController<MoqtObject>

  /** Per-track handlers for received datagrams. */
  #datagramTrackHandlers: Map<string, (obj: MoqtObject) => void> = new Map()

  /**
   * Stream of all received MoqtObjects from datagrams across all tracks
   * Consumer should filter by fullTrackName as needed
   *
   * WARNING: Only one reader should be active. For multiple subscribers,
   * use subscribeToTrackDatagrams() instead
   */
  readonly receivedDatagramObjects: ReadableStream<MoqtObject>

  // Constructor & Factory

  /**
   * Allocate the next client-originated request id using the even/odd stride pattern
   */
  get #nextClientRequestId(): bigint {
    const id = this.#dontUseRequestId
    this.#dontUseRequestId += 2n
    return id
  }

  /**
   * Gets the current server setup configuration
   */
  get serverSetup(): ServerSetup {
    return this.#serverSetup
  }

  /**
   * Returns true if datagram support is currently active
   */
  get isDatagramsEnabled(): boolean {
    return this.#isReceivingDatagrams
  }

  /**
   * Guard that throws if the client has been destroyed
   */
  #ensureActive() {
    if (this.#isDestroyed) throw new MOQtailError('MOQtailClient is destroyed and cannot be used.')
  }

  private constructor() {
    // Create a stream for received datagram objects
    this.receivedDatagramObjects = new ReadableStream<MoqtObject>({
      start: (controller) => {
        this.#receivedDatagramObjectController = controller
      },
      cancel: () => this.stopDatagrams(),
    })
  }

  /**
   * Establishes a new MOQtailClient session over WebTransport and performs the MOQT setup handshake.
   *
   * @param args - {@link MOQtailClientOptions}
   * @returns Promise resolving to a ready MOQtailClient instance.
   * @throws ProtocolViolationError If the server sends an unexpected or invalid message during setup.
   *
   * @example Minimal connection
   * ```ts
   * const client = await MOQtailClient.new({
   *   url: 'https://relay.example.com/transport',
   *   supportedVersions: [0xff00000b]
   * });
   * ```
   *
   * @example With datagram support and callbacks
   * ```ts
   * const client = await MOQtailClient.new({
   *   url,
   *   supportedVersions: [0xff00000b],
   *   enableDatagrams: true,
   *   callbacks: {
   *     onMessageSent: msg => console.log('Sent:', msg),
   *     onDatagramReceived: data => console.log('Datagram:', data),
   *     onSessionTerminated: reason => console.warn('Session ended:', reason)
   *   }
   * });
   * ```
   */
  static async new(args: MOQtailClientOptions): Promise<MOQtailClient> {
    const {
      url,
      supportedVersions,
      setupParameters,
      transportOptions,
      dataStreamTimeoutMs,
      controlStreamTimeoutMs,
      enableDatagrams,
      callbacks,
    } = args

    const client = new MOQtailClient()

    client.webTransport = new WebTransport(url, transportOptions)
    await client.webTransport.ready

    try {
      // Set up callbacks
      if (callbacks?.onMessageSent) client.onMessageSent = callbacks.onMessageSent
      if (callbacks?.onMessageReceived) client.onMessageReceived = callbacks.onMessageReceived
      if (callbacks?.onSessionTerminated) client.onSessionTerminated = callbacks.onSessionTerminated
      if (callbacks?.onDatagramReceived) client.onDatagramReceived = callbacks.onDatagramReceived
      if (callbacks?.onDatagramSent) client.onDatagramSent = callbacks.onDatagramSent

      if (dataStreamTimeoutMs) client.dataStreamTimeoutMs = dataStreamTimeoutMs
      if (controlStreamTimeoutMs) client.controlStreamTimeoutMs = controlStreamTimeoutMs

      // Control stream should have the highest priority
      const biStream = await client.webTransport.createBidirectionalStream({ sendOrder: Number.MAX_SAFE_INTEGER })
      client.controlStream = ControlStream.new(
        biStream,
        client.controlStreamTimeoutMs,
        client.onMessageSent,
        client.onMessageReceived,
      )

      const params = setupParameters ? setupParameters.build() : new SetupParameters().build()
      const clientSetup = new ClientSetup(supportedVersions, params)
      client.controlStream.send(clientSetup)

      const reader = client.controlStream.stream.getReader()
      const { value: response, done } = await reader.read()
      if (done) throw new ProtocolViolationError('MOQtailClient.new', 'Stream closed after client setup')
      if (!(response instanceof ServerSetup))
        throw new ProtocolViolationError('MOQtailClient.new', 'Expected server setup after client setup')

      client.#serverSetup = response
      reader.releaseLock()

      // Start background loops
      client.#handleIncomingControlMessages()
      client.#acceptIncomingUniStreams()

      // Optionally enable datagram support
      if (enableDatagrams) {
        await client.startDatagrams()
      }

      return client
    } catch (error) {
      await client.disconnect(
        new InternalError('MOQtailClient.new', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Start receiving datagrams from the WebTransport connection.
   * Must be called before datagrams can be received (unless enableDatagrams: true was set in options).
   *
   * @throws MOQtailError if client is destroyed or datagrams already started
   */
  async startDatagrams(): Promise<void> {
    this.#ensureActive()

    if (this.#isReceivingDatagrams) {
      console.warn('[MOQtailClient] Datagrams already started')
      return
    }

    console.log('[MOQtailClient] Starting datagram support...')
    this.#datagramReader = this.webTransport.datagrams.readable.getReader()
    this.#datagramWriter = this.webTransport.datagrams.writable.getWriter()
    this.#isReceivingDatagrams = true

    // Start background datagram reception
    this.#acceptIncomingDatagrams()
    console.log('[MOQtailClient] Datagram support started')
  }

  /**
   * Stop receiving datagrams and release resources.
   * Idempotent - safe to call multiple times.
   */
  async stopDatagrams(): Promise<void> {
    if (!this.#isReceivingDatagrams) return

    console.log('[MOQtailClient] Stopping datagram support...')
    this.#isReceivingDatagrams = false
    this.#datagramTrackHandlers.clear()

    if (this.#datagramReader) {
      await this.#datagramReader.cancel().catch(() => {})
      this.#datagramReader.releaseLock()
      this.#datagramReader = undefined
    }

    if (this.#datagramWriter) {
      await this.#datagramWriter.close().catch(() => {})
      this.#datagramWriter = undefined
    }

    if (this.#receivedDatagramObjectController) {
      try {
        this.#receivedDatagramObjectController.close()
      } catch {
        // Already closed
      }
    }
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
   * const unsubscribe = client.subscribeToTrackDatagrams(trackAlias, (obj) => {
   *   console.log('Received datagram:', obj.payload);
   * });
   * // Later: unsubscribe();
   * ```
   */
  subscribeToTrackDatagrams(trackAlias: bigint, handler: (obj: MoqtObject) => void): () => void {
    const key = trackAlias.toString()
    console.log(`[MOQtailClient] Registering datagram handler for trackAlias=${trackAlias}`)
    this.#datagramTrackHandlers.set(key, handler)

    return () => {
      console.log(`[MOQtailClient] Unregistering datagram handler for trackAlias=${trackAlias}`)
      this.#datagramTrackHandlers.delete(key)
    }
  }

  /**
   * Unsubscribe from datagram delivery for a specific track.
   *
   * @param trackAlias - Track alias to unsubscribe from
   */
  unsubscribeFromTrackDatagrams(trackAlias: bigint): void {
    const key = trackAlias.toString()
    this.#datagramTrackHandlers.delete(key)
  }

  /**
   * Create a datagram sender for a specific track.
   *
   * @param trackAlias - Track alias for outgoing datagrams
   * @returns SendDatagramStream for writing MoqtObjects as datagrams
   * @throws MOQtailError if datagram writer not initialized (call startDatagrams() first)
   *
   * @example
   * ```ts
   * const sender = client.createDatagramSender(trackAlias);
   * await sender.write(moqtObject);
   * ```
   */
  createDatagramSender(trackAlias: bigint): SendDatagramStream {
    this.#ensureActive()

    if (!this.#datagramWriter) {
      throw new MOQtailError(
        'Datagrams not started. Call startDatagrams() first or set enableDatagrams: true in options.',
      )
    }

    console.log(`[MOQtailClient] Creating datagram sender for trackAlias=${trackAlias}`)
    return SendDatagramStream.fromWriter(this.#datagramWriter, trackAlias, this.onDatagramSent)
  }

  /**
   * Send a single MoqtObject as a datagram.
   * Convenience method for one-off datagram sends.
   *
   * @param trackAlias - Track alias for this object
   * @param object - MoqtObject to send
   * @throws MOQtailError if datagram writer not initialized
   *
   * @example
   * ```ts
   * await client.sendDatagram(trackAlias, moqtObject);
   * ```
   */
  async sendDatagram(trackAlias: bigint, object: MoqtObject): Promise<void> {
    this.#ensureActive()

    if (!this.#datagramWriter) {
      throw new MOQtailError(
        'Datagrams not started. Call startDatagrams() first or set enableDatagrams: true in options.',
      )
    }

    let serialized: Uint8Array

    if (object.hasStatus()) {
      const datagramStatus = object.tryIntoDatagramStatus(trackAlias)
      serialized = datagramStatus.serialize().toUint8Array()
      if (this.onDatagramSent) this.onDatagramSent(datagramStatus)
    } else if (object.hasPayload()) {
      const datagramObject = object.tryIntoDatagramObject(trackAlias)
      serialized = datagramObject.serialize().toUint8Array()
      if (this.onDatagramSent) this.onDatagramSent(datagramObject)
    } else {
      throw new InternalError('sendDatagram', 'MoqtObject must have payload or status')
    }

    await this.#datagramWriter.write(serialized)
  }

  /**
   * Background loop that receives and parses incoming datagrams.
   */
  async #acceptIncomingDatagrams(): Promise<void> {
    console.log('[MOQtailClient] Starting datagram reception loop...')

    try {
      while (this.#isReceivingDatagrams && this.#datagramReader) {
        const { done, value: datagramBytes } = await this.#datagramReader.read()

        if (done) {
          console.log('[MOQtailClient] Datagram reader done, stopping reception')
          this.#isReceivingDatagrams = false
          if (this.#receivedDatagramObjectController) {
            try {
              this.#receivedDatagramObjectController.close()
            } catch {
              // Already closed
            }
          }
          break
        }

        if (!datagramBytes || datagramBytes.length === 0) {
          continue
        }

        const firstByte = datagramBytes[0]!

        try {
          // Parse datagram (peek at first byte to determine type)
          const isStatus = firstByte === 0x02 || firstByte === 0x03

          let moqtObject: MoqtObject
          let trackAlias: bigint

          if (isStatus) {
            // DatagramStatus (0x02 or 0x03)
            const datagramStatus = DatagramStatus.deserialize(new FrozenByteBuffer(datagramBytes))
            trackAlias = datagramStatus.trackAlias

            if (this.onDatagramReceived) {
              this.onDatagramReceived(datagramStatus)
            }

            const fullTrackName = this.#resolveTrackAlias(trackAlias)
            moqtObject = MoqtObject.fromDatagramStatus(datagramStatus, fullTrackName)
          } else {
            // DatagramObject (0x00 or 0x01)
            const datagramObject = DatagramObject.deserialize(new FrozenByteBuffer(datagramBytes))
            trackAlias = datagramObject.trackAlias

            if (this.onDatagramReceived) {
              this.onDatagramReceived(datagramObject)
            }

            const fullTrackName = this.#resolveTrackAlias(trackAlias)
            moqtObject = MoqtObject.fromDatagramObject(datagramObject, fullTrackName)
          }

          // Dispatch to track-specific handler if registered
          const trackKey = trackAlias.toString()
          const handler = this.#datagramTrackHandlers.get(trackKey)
          if (handler) {
            try {
              handler(moqtObject)
            } catch (handlerError) {
              console.warn('[MOQtailClient] Datagram track handler error:', handlerError)
            }
          }

          // Also enqueue to the general stream
          if (this.#receivedDatagramObjectController) {
            try {
              this.#receivedDatagramObjectController.enqueue(moqtObject)
            } catch {
              // Stream closed
            }
          }
        } catch (error) {
          // Log but don't break - individual datagrams may be corrupt/unknown
          console.warn('[MOQtailClient] Failed to parse datagram:', error)
          continue
        }
      }
    } catch (error) {
      console.error('[MOQtailClient] Datagram reception error:', error)
      if (this.#receivedDatagramObjectController) {
        try {
          this.#receivedDatagramObjectController.error(error)
        } catch {
          // Already errored/closed
        }
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
      const requestId = this.subscriptionAliasMap.get(trackAlias)
      if (requestId !== undefined) {
        return this.requestIdMap.getNameByRequestId(requestId)
      }
      return FullTrackName.tryNew('unknown', `track-${trackAlias}`)
    } catch {
      return FullTrackName.tryNew('unknown', `track-${trackAlias}`)
    }
  }

  /**
   * Gracefully terminates this session and releases underlying WebTransport resources.
   *
   * @param reason - Optional application-level reason
   * @returns Promise that resolves once shutdown logic completes. Subsequent calls are safe no-ops.
   */
  async disconnect(reason?: unknown) {
    console.log('disconnect', reason)
    if (this.#isDestroyed) return
    this.#isDestroyed = true

    // Stop datagrams first
    await this.stopDatagrams()

    if (!this.webTransport.closed) this.webTransport.close()
    if (this.onSessionTerminated)
      this.onSessionTerminated(
        new InternalError('MOQtailClient.disconnect', reason instanceof Error ? reason.message : String(reason)),
      )
  }

  /**
   * Registers or updates a Track definition for local publishing or serving.
   */
  addOrUpdateTrack(track: Track) {
    this.#ensureActive()
    this.trackSources.set(track.fullTrackName.toString(), track)
  }

  /**
   * Removes a previously registered Track from this client's local catalog.
   */
  removeTrack(track: Track) {
    this.#ensureActive()
    this.trackSources.delete(track.fullTrackName.toString())
  }

  /**
   * Subscribes to a track and returns a stream of MoqtObjects.
   */
  async subscribe(
    args: SubscribeOptions,
  ): Promise<SubscribeError | { requestId: bigint; stream: ReadableStream<MoqtObject> }> {
    this.#ensureActive()
    try {
      let { fullTrackName, priority, groupOrder, forward, filterType, parameters, startLocation, endGroup } = args

      let msg: Subscribe
      if (typeof endGroup === 'number') endGroup = BigInt(endGroup)
      if (!parameters) parameters = new VersionSpecificParameters()

      switch (filterType) {
        case FilterType.LatestObject:
          msg = Subscribe.newLatestObject(
            this.#nextClientRequestId,
            fullTrackName,
            priority,
            groupOrder,
            forward,
            parameters.build(),
          )
          break
        case FilterType.NextGroupStart:
          msg = Subscribe.newNextGroupStart(
            this.#nextClientRequestId,
            fullTrackName,
            priority,
            groupOrder,
            forward,
            parameters.build(),
          )
          break
        case FilterType.AbsoluteStart:
          if (!startLocation)
            throw new ProtocolViolationError(
              'MOQtailClient.subscribe',
              'FilterType.AbsoluteStart must have a start location',
            )
          msg = Subscribe.newAbsoluteStart(
            this.#nextClientRequestId,
            fullTrackName,
            priority,
            groupOrder,
            forward,
            startLocation,
            parameters.build(),
          )
          break
        case FilterType.AbsoluteRange:
          if (startLocation === undefined || endGroup === undefined)
            throw new ProtocolViolationError(
              'MOQtailClient.subscribe',
              'FilterType.AbsoluteRange must have a start location and an end group',
            )
          if (endGroup > 0 && startLocation.group >= endGroup)
            throw new ProtocolViolationError('MOQtailClient.subscribe', 'End group must be greater than start group')

          msg = Subscribe.newAbsoluteRange(
            this.#nextClientRequestId,
            fullTrackName,
            priority,
            groupOrder,
            forward,
            startLocation,
            endGroup,
            parameters.build(),
          )
          break
      }

      const request = new SubscribeRequest(msg)
      this.requests.set(request.requestId, request)
      this.requestIdMap.addMapping(request.requestId, request.fullTrackName)
      await this.controlStream.send(msg)
      const response = await request

      if (response instanceof SubscribeError) {
        this.requests.delete(request.requestId)
        this.requestIdMap.removeMappingByRequestId(request.requestId)
        return response
      } else {
        this.subscriptions.set(response.trackAlias, request)
        this.subscriptionAliasMap.set(request.requestId, response.trackAlias)
        return { requestId: msg.requestId, stream: request.stream }
      }
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.subscribe', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Stops an active subscription identified by its original SUBSCRIBE requestId.
   */
  async unsubscribe(requestId: bigint | number): Promise<void> {
    this.#ensureActive()
    if (typeof requestId === 'number') requestId = BigInt(requestId)
    let cleanupData: { requestId: bigint; trackAlias: bigint; subscription: SubscribeRequest } | null = null

    try {
      if (this.requests.has(requestId)) {
        const subscription = this.requests.get(requestId)!
        if (subscription instanceof SubscribeRequest) {
          const trackAlias = this.subscriptionAliasMap.get(requestId)!
          cleanupData = { requestId, trackAlias, subscription }

          await this.controlStream.send(new Unsubscribe(requestId))
          subscription.unsubscribe()
        }
      }
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.unsubscribe', error instanceof Error ? error.message : String(error)),
      )
      throw error
    } finally {
      if (cleanupData) {
        this.requests.delete(cleanupData.requestId)
        this.subscriptions.delete(cleanupData.trackAlias)
        this.requestIdMap.removeMappingByRequestId(cleanupData.requestId)
      }
    }
  }

  /**
   * Narrows or updates an active subscription window and/or relay forwarding behavior.
   */
  async subscribeUpdate(args: SubscribeUpdateOptions): Promise<void> {
    this.#ensureActive()
    let { subscriptionRequestId, priority, forward, parameters, startLocation, endGroup } = args

    if (endGroup && startLocation.group >= endGroup)
      throw new ProtocolViolationError('MOQtailClient.subscribeUpdate', 'End group must be greater than start group')

    try {
      if (this.requests.has(subscriptionRequestId)) {
        const request = this.requests.get(subscriptionRequestId)!
        if (request instanceof SubscribeRequest) {
          const trackAlias = this.subscriptionAliasMap.get(subscriptionRequestId)
          if (!trackAlias)
            throw new InternalError('MOQtailClient.subscribeUpdate', 'Request exists but track alias mapping does not')
          const subscription = this.subscriptions.get(trackAlias)
          if (!subscription)
            throw new InternalError('MOQtailClient.subscribeUpdate', 'Request exists but subscription does not')

          if (!parameters) parameters = new VersionSpecificParameters()
          const requestId = this.#nextClientRequestId
          const msg = new SubscribeUpdate(
            requestId,
            subscriptionRequestId,
            startLocation,
            endGroup,
            priority,
            forward,
            parameters.build(),
          )
          subscription.update(msg)
          await this.controlStream.send(msg)
        }
      }
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.subscribeUpdate', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Performs a FETCH operation and returns a stream of MoqtObjects.
   */
  async fetch(args: FetchOptions): Promise<FetchError | { requestId: bigint; stream: ReadableStream<MoqtObject> }> {
    this.#ensureActive()
    try {
      const { priority, groupOrder, typeAndProps, parameters } = args
      if (priority < 0 || priority > 255)
        throw new ProtocolViolationError(
          'MOQtailClient.fetch',
          `subscriberPriority: ${priority} must be in range of [0-255]`,
        )

      const params = parameters ? parameters.build() : new VersionSpecificParameters().build()
      let msg: Fetch
      let joiningRequest: MOQtailRequest | undefined
      const requestId = this.#nextClientRequestId

      switch (typeAndProps.type) {
        case FetchType.StandAlone:
          msg = new Fetch(
            requestId,
            priority,
            groupOrder,
            { type: typeAndProps.type, props: typeAndProps.props },
            params,
          )
          break

        case FetchType.Relative:
          joiningRequest = this.requests.get(typeAndProps.props.joiningRequestId)
          if (!(joiningRequest instanceof SubscribeRequest))
            throw new ProtocolViolationError(
              'MOQtailClient.fetch',
              `No subscribe request for the given joiningRequestId: ${typeAndProps.props.joiningRequestId}`,
            )
          msg = new Fetch(
            requestId,
            priority,
            groupOrder,
            { type: typeAndProps.type, props: typeAndProps.props },
            params,
          )
          break

        case FetchType.Absolute:
          joiningRequest = this.requests.get(typeAndProps.props.joiningRequestId)
          if (!(joiningRequest instanceof SubscribeRequest))
            throw new ProtocolViolationError(
              'MOQtailClient.fetch',
              `No subscribe request for the given joiningRequestId: ${typeAndProps.props.joiningRequestId}`,
            )
          msg = new Fetch(
            requestId,
            priority,
            groupOrder,
            { type: typeAndProps.type, props: typeAndProps.props },
            params,
          )
          break
      }

      const request = new FetchRequest(msg)
      this.requests.set(msg.requestId, request)
      await this.controlStream.send(msg)
      const response = await request

      if (response instanceof FetchError) {
        this.requests.delete(msg.requestId)
        return response
      } else {
        const stream = request.stream
        return { requestId: msg.requestId, stream }
      }
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.fetch', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Sends a FETCH_CANCEL for an active fetch request.
   */
  async fetchCancel(requestId: bigint | number) {
    this.#ensureActive()
    try {
      if (typeof requestId === 'number') requestId = BigInt(requestId)
      const request = this.requests.get(requestId)
      if (request) {
        if (request instanceof FetchRequest) {
          this.controlStream.send(new FetchCancel(requestId))
        }
      }
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.fetchCancel', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Publish a namespace to the relay
   */
  async publishNamespace(trackNamespace: Tuple, parameters?: VersionSpecificParameters) {
    this.#ensureActive()
    try {
      const params = parameters ? parameters.build() : new VersionSpecificParameters().build()
      const msg = new PublishNamespace(this.#nextClientRequestId, trackNamespace, params)
      const request = new PublishNamespaceRequest(msg.requestId, msg)
      this.requests.set(msg.requestId, request)
      this.controlStream.send(msg)
      const response = await request
      if (response instanceof PublishNamespaceOk) this.announcedNamespaces.add(msg.trackNamespace)
      this.requests.delete(msg.requestId)
      return response
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.publishNamespace', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Send a PublishNamespaceDone to signal the end of publishing for a namespace.
   */
  async publishNamespaceDone(trackNamespace: Tuple) {
    this.#ensureActive()
    try {
      const msg = new PublishNamespaceDone(trackNamespace)
      this.announcedNamespaces.delete(msg.trackNamespace)
      await this.controlStream.send(msg)
    } catch (err) {
      await this.disconnect()
      throw err
    }
  }

  /**
   * Cancel a previously sent PUBLISH_NAMESPACE request.
   */
  async publishNamespaceCancel(msg: PublishNamespaceCancel) {
    this.#ensureActive()
    try {
      await this.controlStream.send(msg)
    } catch (error) {
      await this.disconnect(
        new InternalError(
          'MOQtailClient.publishNamespaceCancel',
          error instanceof Error ? error.message : String(error),
        ),
      )
      throw error
    }
  }

  /**
   * Subscribe to a namespace
   */
  async subscribeNamespace(msg: SubscribeNamespace) {
    this.#ensureActive()
    try {
      await this.controlStream.send(msg)
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.subscribeNamespace', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Unsubscribe from a namespace
   */
  async unsubscribeNamespace(msg: UnsubscribeNamespace) {
    this.#ensureActive()
    try {
      await this.controlStream.send(msg)
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.unsubscribeNamespace', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /* Background Handlers & Loops */

  /**
   * Background loop that processes incoming control messages
   */
  async #handleIncomingControlMessages(): Promise<void> {
    this.#ensureActive()
    try {
      const reader = this.controlStream.stream.getReader()
      while (true) {
        const { done, value: msg } = await reader.read()
        if (done) throw new MOQtailError('WebTransport session is terminated')
        const handler = getHandlerForControlMessage(msg)
        if (!handler) throw new ProtocolViolationError('MOQtailClient', 'No handler for the received message')
        // Note: Handler expects MOQtailClient but we're MOQtailClient
        // This works because we have the same public interface
        await handler(this as unknown as import('./client').MOQtailClient, msg)
      }
    } catch (error) {
      this.disconnect()
      throw error
    }
  }

  /**
   * Background loop that accepts incoming unidirectional data streams.
   */
  async #acceptIncomingUniStreams() {
    this.#ensureActive()
    const uds = this.webTransport.incomingUnidirectionalStreams
    const reader = uds.getReader()
    let isDone = false

    while (!isDone) {
      try {
        const { done, value: stream } = await reader.read()
        if (done) {
          isDone = true
          throw new MOQtailError('WebTransport session is terminated')
        }
        this.#handleRecvStreams(stream)
      } catch (error) {
        console.log('acceptIncomingUniStreams error', error)
        if (this.#isDestroyed) break
      }
    }
  }

  /*
   * Handler for incoming unidirectional data streams.
   */
  async #handleRecvStreams(incomingUniStream: ReadableStream): Promise<void> {
    this.#ensureActive()
    try {
      const recvStream = await RecvStream.new(incomingUniStream, this.dataStreamTimeoutMs, this.onDataReceived)
      const header = recvStream.header
      const reader = recvStream.stream.getReader()

      if (header instanceof FetchHeader) {
        const request = this.requests.get(header.requestId)
        if (request && request instanceof FetchRequest) {
          let fullTrackName: FullTrackName
          switch (request.message.typeAndProps.type) {
            case FetchType.StandAlone:
              fullTrackName = request.message.typeAndProps.props.fullTrackName
              break
            case FetchType.Relative:
            case FetchType.Absolute: {
              const joiningSubscription = this.requests.get(request.message.typeAndProps.props.joiningRequestId)
              if (joiningSubscription instanceof SubscribeRequest) {
                fullTrackName = joiningSubscription.fullTrackName
                break
              }
              throw new ProtocolViolationError(
                '_handleRecvStreams',
                'No active subscription for given joining request id',
              )
            }
            default:
              throw new ProtocolViolationError('_handleRecvStreams', 'Unknown fetchType')
          }

          try {
            while (true) {
              const { done, value: nextObject } = await reader.read()
              if (done) {
                request.controller?.close()
                break
              }
              if (nextObject) {
                if (nextObject instanceof FetchObject) {
                  const moqtObject = MoqtObject.fromFetchObject(nextObject, fullTrackName)
                  request.controller?.enqueue(moqtObject)
                  continue
                }
                throw new ProtocolViolationError('MOQtailClient', 'Received subgroup object after fetch header')
              }
            }
          } finally {
            reader.releaseLock()
          }
          return
        }
        throw new ProtocolViolationError('MOQtailClient', 'No request for received request id')
      } else {
        const subscription = this.subscriptions.get(header.trackAlias)
        if (subscription) {
          subscription.streamsAccepted++
          let firstObjectId: bigint | null = null

          while (true) {
            const { done, value: nextObject } = await reader.read()
            if (done) {
              break
            }
            if (nextObject) {
              if (nextObject instanceof SubgroupObject) {
                if (!firstObjectId) {
                  firstObjectId = nextObject.objectId
                }
                let subgroupId = header.subgroupId
                switch (header.type) {
                  case SubgroupHeaderType.Type0x10:
                  case SubgroupHeaderType.Type0x11:
                  case SubgroupHeaderType.Type0x18:
                  case SubgroupHeaderType.Type0x19:
                    subgroupId = 0n
                    break
                  case SubgroupHeaderType.Type0x12:
                  case SubgroupHeaderType.Type0x13:
                  case SubgroupHeaderType.Type0x1A:
                  case SubgroupHeaderType.Type0x1B:
                    subgroupId = firstObjectId
                    break
                  case SubgroupHeaderType.Type0x14:
                  case SubgroupHeaderType.Type0x15:
                  case SubgroupHeaderType.Type0x1C:
                  case SubgroupHeaderType.Type0x1D:
                    subgroupId = header.subgroupId!
                }

                const moqtObject = MoqtObject.fromSubgroupObject(
                  nextObject,
                  header.groupId,
                  header.publisherPriority,
                  subgroupId,
                  this.requestIdMap.getNameByRequestId(subscription.requestId),
                )
                if (!subscription.largestLocation) subscription.largestLocation = moqtObject.location
                if (subscription.largestLocation.compare(moqtObject.location) == -1)
                  subscription.largestLocation = moqtObject.location

                subscription.controller?.enqueue(moqtObject)
                continue
              }
              throw new ProtocolViolationError('MOQtailClient', 'Received fetch object after subgroup header')
            }
          }

          // Subscribe Cleanup
          if (subscription.expectedStreams && subscription.expectedStreams === subscription.streamsAccepted) {
            subscription.controller?.close()
            this.subscriptions.delete(header.trackAlias)
            this.requests.delete(subscription.requestId)
          }
          return
        }
        throw new ProtocolViolationError('MOQtailClient', 'No subscription for received track alias')
      }
    } catch (error) {
      throw error
    }
  }
}
