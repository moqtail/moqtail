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

import { ControlRecvStream, ControlSendStream, ControlStream } from './control_stream'
import { RequestStream } from './request_stream'
import {
  PublishNamespace,
  PublishNamespaceDone,
  Namespace,
  NamespaceDone,
  NamespaceSubscribeOptions,
  Setup,
  ControlMessage,
  Fetch,
  FetchOk,
  FetchType,
  FilterType,
  GoAway,
  GroupOrder,
  Subscribe,
  SubscribeNamespace,
  RequestError,
  RequestUpdate,
  UnsubscribeNamespace,
  Publish,
  RequestOk,
  Switch,
  SubscribeOk,
  TrackStatus,
  ControlMessageType,
  SUPPORTED_VERSIONS,
  PublishDone,
} from '../model/control'
import {
  Datagram,
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
import { DeliveryTimeoutExtension, TrackExtension } from '../model/extension_header/track_extension'
import { RecvStream } from './data_stream'
import {
  InternalError,
  Location,
  MOQtailError,
  ProtocolViolationError,
  ReasonPhrase,
  SetupOptions,
  assertNoAuthorityOverWebTransport,
  Tuple,
  MessageParameter,
  Forward,
  SubscriberPriority,
  GroupOrderParam,
  SubscriptionFilter,
} from '../model'
import { PublishNamespaceCancel } from '../model/control/publish_namespace_cancel'
import { Track } from './track/track'
import { PublishNamespaceRequest } from './request/publish_namespace'
import { FetchRequest } from './request/fetch'
import { SubscribeRequest } from './request/subscribe'
import { PublishRequest } from './request/publish'
import { TrackStatusRequest } from './request/track_status'
import { getHandlerForControlMessage, getHandlerForRequestStreamMessage } from './handler/handler'
import { SubscribePublication } from './publication/subscribe'
import { FetchPublication } from './publication/fetch'
import { PublishPublication } from './publication/publish'
import { random60bitId } from './util/random_id'
import { isValidTrackAlias } from './util/validators'
import {
  MOQtailRequest,
  SubscribeOptions,
  SubscribeUpdateOptions,
  FetchOptions,
  MOQtailClientOptions,
  SwitchOptions,
  EarlyDiscardPolicyConfig,
} from './types'
import { SendDatagramStream } from './datagram_stream'
import { logger, LogLevel, setLogLevel, setLogEnabledModules } from '../util/logger'

/**
 * @public
 * Represents a Media Over QUIC Transport (MOQT) client session.
 *
 * Use {@link MOQtailClient.new} to establish a connection and perform MOQT operations such as subscribing to tracks,
 * fetching historical data, announcing tracks for publication, and managing session lifecycle.
 *
 * Once initialized, the client provides high-level methods for MOQT requests and publishing. If a protocol violation
 * occurs, the client will terminate and must be re-initialized.
 *
 * ## Usage
 *
 * ### Connect and Subscribe to a Track
 * ```ts
 * const client = await MOQtailClient.new({ url });
 * const result = await client.subscribe({
 *   fullTrackName,
 *   filterType: FilterType.LatestObject,
 *   forward: true,
 *   groupOrder: GroupOrder.Original,
 *   priority: 0
 * });
 * if (!(result instanceof RequestError)) {
 *   for await (const object of result.stream) {
 *     // Consume MOQT objects
 *   }
 * }
 * ```
 *
 * ### Publish a namespace for Publishing
 * ```ts
 * const client = await MOQtailClient.new({ url });
 * const publishNamespaceResult = await client.publishNamespace(["camera", "main"]);
 * if (!(publishNamespaceResult instanceof RequestError)) {
 *   // Ready to publish objects under this namespace
 * }
 * ```
 *
 * ### Graceful Shutdown
 * ```ts
 * await client.disconnect();
 * ```
 */
export class MOQtailClient {
  /**
   * Namespace prefixes (tuples) the peer has requested announce notifications for via SUBSCRIBE_NAMESPACE.
   * Used to decide which locally issued ANNOUNCE messages should be forwarded (future optimization: prefix trie).
   */
  readonly peerSubscribeNamespace = new Set<Tuple>()
  /**
   * Namespace prefixes this client has subscribed to (issued SUBSCRIBE_NAMESPACE). Enables automatic filtering
   * of incoming PUBLISH_NAMESPACE / PUBLISH_NAMESPACE_DONE. Maintained locally; no dedupe of overlapping / shadowing prefixes yet.
   */
  readonly subscribedAnnounces = new Set<Tuple>()
  /**
   * Track namespaces this client has successfully announced (received ANNOUNCE_OK). Source of truth for
   * deciding what to PUBLISH_NAMESPACE_DONE on teardown or targeted withdrawal.(future optimization: prefix trie).
   */
  readonly announcedNamespaces = new Set<Tuple>()
  /**
   * Locally registered track definitions keyed by full track name string. Populated via addOrUpdateTrack.
   * Does not imply the track has been announced or has active publications.
   */
  readonly trackSources: Map<string, Track> = new Map()
  /**
   * All in‑flight request objects keyed by requestId (SUBSCRIBE, FETCH, ANNOUNCE, etc). Facilitates lookup
   * when responses / data arrive. Entries are removed on completion or error.
   */
  readonly requests: Map<bigint, MOQtailRequest> = new Map()
  /**
   * Active publications (SUBSCRIBE or FETCH) keyed by requestId to manage object stream controllers and lifecycle.
   * Subset / specialization view of `requests`.
   */
  readonly publications: Map<bigint, SubscribePublication | FetchPublication | PublishPublication> = new Map()
  /**
   * Active SUBSCRIBE request wrappers keyed by track alias for rapid alias -\> subscription resolution during
   * incoming unidirectional data handling.
   */
  readonly subscriptions: Map<bigint, any> = new Map()
  /**
   * Bidirectional track alias \<-\> subscription requestId mapping
   */
  readonly subscriptionAliasMap: Map<bigint, bigint> = new Map()
  /**
   * Bidirectional requestId \<-\> full track name mapping to reconstruct metadata for incoming objects.
   */
  readonly requestIdMap: RequestIdMap = new RequestIdMap()
  /**
   * Maps track aliases to full track names for quick resolution during data handling.
   */
  readonly aliasFullTrackNameMap: Map<bigint, FullTrackName> = new Map()
  /**
   * Pending state updates keyed by requestId and applied once a new track alias is seen.
   * Used to avoid premature state updates.
   */
  readonly pendingStateUpdates: Map<bigint, (newTrackAlias: bigint) => boolean> = new Map()

  /**
   * The bidirectional request stream each locally issued request runs on, keyed by the
   * requestId of the message that opened it (draft-18 §3.3.2). The stream stays open for
   * the request's lifetime: responses and follow-ups arrive on it, updates are written to
   * it, and closing it is how the request is cancelled.
   */
  readonly #requestStreams: Map<bigint, RequestStream> = new Map()

  /**
   * Namespace path -\> the requestId that announced or subscribed to it, so the
   * namespace-keyed APIs ({@link MOQtailClient.publishNamespaceDone}) can find the
   * stream to close.
   */
  readonly #namespaceRequestIds: Map<string, bigint> = new Map()

  /** Underlying WebTransport session (set after successful construction in MOQtailClient.new). */
  webTransport!: WebTransport
  /** Validated Setup message the server sent back during handshake (protocol parameters negotiated). */
  #serverSetup!: Setup
  /** Outgoing / incoming control message stream pair. */
  controlStream!: ControlStream
  /** Reader over incoming uni streams: the control recv half first, data streams after. */
  #incomingUniStreams!: ReadableStreamDefaultReader<ReadableStream<Uint8Array>>
  /** Timeout (ms) applied to reading incoming data streams; undefined =\> no explicit timeout. */
  dataStreamTimeoutMs?: number
  /** Timeout (ms) for control stream read operations; undefined =\> no explicit timeout. */
  controlStreamTimeoutMs?: number

  /** Flag indicating the client has been disconnected/destroyed and cannot accept further API calls. */
  #isDestroyed = false
  /** Internal monotonically increasing client-assigned request id counter (even/odd parity scheme advances by 2). */
  #dontUseRequestId: bigint = 0n
  /** Active early discard policy; undefined means no per-stream deadline is applied. */
  #earlyDiscardPolicy: EarlyDiscardPolicyConfig | undefined

  /**
   * TODO: onNamespaceAnnounced may be a better name
   * Fired when an PUBLISH_NAMESPACE control message is processed for a track namespace.
   * Use to update UI or trigger discovery logic.
   * Discovery event.
   */
  onNamespacePublished?: (msg: PublishNamespace) => void

  /**
   * Fired when an PUBLISH_NAMESPACE_DONE control message is processed for a namespace.
   * Use to remove tracks from UI or stop discovery.
   * Discovery event.
   */
  onNamespaceDone?: (msg: PublishNamespaceDone) => void

  /**
   * Fired on GOAWAY reception signaling graceful session wind-down.
   * Use to prepare for disconnect or cleanup.
   * Lifecycle handler.
   */
  onGoaway?: (msg: GoAway) => void

  /**
   * Fired if the underlying WebTransport session fails (ready -\> closed prematurely).
   * Use to log or alert on transport errors.
   * Lifecycle/error handler.
   */
  onWebTransportFail?: () => void

  /**
   * Fired exactly once when the client transitions to terminated (disconnect).
   * Use to clean up resources or notify user.
   * Lifecycle handler.
   */
  onSessionTerminated?: (reason?: unknown) => void

  /**
   * Invoked after each outbound control message is sent.
   * Use for logging or analytics.
   * Informational event.
   */
  onMessageSent?: (msg: ControlMessage) => void

  /**
   * Invoked upon receiving each inbound control message before handling.
   * Use for logging or debugging.
   * Informational event.
   */
  onMessageReceived?: (msg: ControlMessage) => void

  /**
   * Invoked for each decoded data object/header arriving on a uni stream (fetch or subgroup).
   * Use to process or display incoming media/data.
   * Informational event.
   */
  onDataReceived?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void

  /**
   * Invoked after enqueuing each outbound data object/header.
   * Reserved for future use.
   * Informational event.
   */
  onDataSent?: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void

  /**
   * General-purpose error callback for surfaced exceptions not thrown to caller synchronously.
   * Use to log or display errors.
   * Error handler.
   */
  onError?: (er: unknown) => void

  /** Invoked for each decoded datagram object/status arriving. */
  onDatagramReceived?: (data: Datagram) => void

  /** Invoked after enqueuing each outbound datagram object/status. */
  onDatagramSent?: (data: Datagram) => void

  /** Fired when an inbound PUBLISH control message is received. */
  onPeerPublish?: (msg: Publish, stream: ReadableStream<MoqtObject>) => void

  /** Fired when an inbound PUBLISH_DONE control message is received. */
  onPeerPublishDone?: (msg: PublishDone) => void

  /** Fired when an inbound SUBSCRIBE_NAMESPACE control message is received. */
  onPeerSubscribeNamespace?: (msg: SubscribeNamespace) => void

  /** Fired when a NAMESPACE message arrives on a SUBSCRIBE_NAMESPACE bi-stream (prefix + suffix). */
  onPeerNamespace?: (prefix: Tuple, suffix: Tuple) => void

  /** Fired when a NAMESPACE_DONE message arrives on a SUBSCRIBE_NAMESPACE bi-stream (prefix + suffix). */
  onPeerNamespaceDone?: (prefix: Tuple, suffix: Tuple) => void

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

  /**
   * Allocate the next client-originated request id using the even/odd stride pattern (increments by 2).
   * Ensures uniqueness within the session and leaves space for peer-assigned ids if parity strategy is employed.
   */
  get #nextClientRequestId(): bigint {
    const id = this.#dontUseRequestId
    this.#dontUseRequestId += 2n
    return id
  }

  /**
   * Generates a safe, sequential local request ID for tracking pushed/incoming tracks.
   */
  allocatePseudoRequestId(): bigint {
    return this.#nextClientRequestId
  }

  /**
   * Gets the current server setup configuration.
   *
   * @returns The {@link Setup} instance the server sent back during handshake.
   */
  get serverSetup(): Setup {
    return this.#serverSetup
  }

  /**
   * Returns true if datagram support is currently active
   */
  get isDatagramsEnabled(): boolean {
    return this.#isReceivingDatagrams
  }

  /**
   * Sets the global log level for all moqtail-ts loggers.
   * @param level - The minimum {@link LogLevel} to output. Use `LogLevel.NONE` to silence all logs.
   */
  static setLogLevel(level: LogLevel): void {
    setLogLevel(level)
  }

  /**
   * Restricts log output to the specified module names. Pass `null` to allow all modules.
   * @param modules - Array of module name strings, or `null` to enable all modules.
   */
  static setLogEnabledModules(modules: string[] | null): void {
    setLogEnabledModules(modules)
  }

  /**
   * Guard that throws if the client has been destroyed (disconnect already called). Used at start of public APIs
   * to fail fast rather than perform partial operations on a torn-down session.
   * @throws MOQtailError when #isDestroyed is true.
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
   * Establishes a new {@link MOQtailClient} session over WebTransport and performs the MOQT setup handshake.
   *
   * @param args - {@link MOQtailClientOptions}
   *
   * @returns Promise resolving to a ready {@link MOQtailClient} instance.
   *
   * @throws :{@link ProtocolViolationError} If the server sends an unexpected or invalid message during setup.
   *
   * @example Minimal connection
   * ```ts
   * const client = await MOQtailClient.new({
   *   url: 'https://relay.example.com/transport'
   * });
   * ```
   *
   * @example With callbacks and options
   * ```ts
   * const client = await MOQtailClient.new({
   *   url,
   *   setupOptions: new SetupOptions().addPath('/live'),
   *   transportOptions: { congestionControl: 'default' },
   *   dataStreamTimeoutMs: 5000,
   *   controlStreamTimeoutMs: 2000,
   *   enableDatagrams: true,
   *   callbacks: {
   *     onMessageSent: msg => console.log('Sent:', msg),
   *     onMessageReceived: msg => console.log('Received:', msg),
   *     onSessionTerminated: reason => console.warn('Session ended:', reason),
   *     onDatagramReceived: data => console.log('Datagram:', data),
   *   }
   * });
   * ```
   */
  static async new(args: MOQtailClientOptions): Promise<MOQtailClient> {
    let {
      url,
      setupOptions,
      transportOptions,
      dataStreamTimeoutMs,
      controlStreamTimeoutMs,
      enableDatagrams,
      callbacks,
    } = args
    const client = new MOQtailClient()

    // send supported versions
    // The protocols are sent in wt-available-protocols header
    if (!transportOptions) {
      transportOptions = { protocols: [] }
    }
    if (!transportOptions.protocols) {
      transportOptions = { ...transportOptions, protocols: [...SUPPORTED_VERSIONS] }
    } else {
      transportOptions.protocols.push(...SUPPORTED_VERSIONS)
    }

    logger.log('MOQtailClient', 'transportOptions', transportOptions)

    client.webTransport = new WebTransport(url, transportOptions)

    await client.webTransport.ready
    try {
      if (callbacks?.onMessageSent) client.onMessageSent = callbacks.onMessageSent
      if (callbacks?.onMessageReceived) client.onMessageReceived = callbacks.onMessageReceived
      if (callbacks?.onSessionTerminated) client.onSessionTerminated = callbacks.onSessionTerminated
      if (callbacks?.onDatagramReceived) client.onDatagramReceived = callbacks.onDatagramReceived
      if (callbacks?.onDatagramSent) client.onDatagramSent = callbacks.onDatagramSent

      if (dataStreamTimeoutMs) client.dataStreamTimeoutMs = dataStreamTimeoutMs
      if (controlStreamTimeoutMs) client.controlStreamTimeoutMs = controlStreamTimeoutMs

      // The control plane is a pair of uni streams. Open our send half and write
      // SETUP first so it goes out without waiting on the server's half, which the
      // relay only opens after accepting ours. Control streams get the highest priority.
      const sendStream = await client.webTransport.createUnidirectionalStream({
        sendOrder: Number.MAX_SAFE_INTEGER,
      })
      const sendHalf = new ControlSendStream(sendStream, client.onMessageSent)
      const params = setupOptions ? setupOptions.build() : new SetupOptions().build()
      assertNoAuthorityOverWebTransport(params)
      await sendHalf.send(new Setup(params))

      // The server's control stream is the first uni stream it opens; every later
      // one is a data stream, so the same reader is reused by #acceptIncomingUniStreams.
      client.#incomingUniStreams = client.webTransport.incomingUnidirectionalStreams.getReader()
      const { value: recvStream, done: recvStreamDone } = await client.#incomingUniStreams.read()
      if (recvStreamDone || !recvStream)
        throw new ProtocolViolationError('MOQtailClient.new', 'Session closed before the server control stream')
      const recvHalf = new ControlRecvStream(recvStream, client.controlStreamTimeoutMs, client.onMessageReceived)
      client.controlStream = new ControlStream(sendHalf, recvHalf)

      const reader = client.controlStream.stream.getReader()
      const { value: response, done } = await reader.read()
      if (done) throw new ProtocolViolationError('MOQtailClient.new', 'Stream closed after client setup')
      if (!(response instanceof Setup))
        throw new ProtocolViolationError('MOQtailClient.new', 'Expected setup as the first control message')

      client.#serverSetup = response
      reader.releaseLock()

      client.#handleIncomingControlMessages()
      client.#acceptIncomingUniStreams()
      client.#acceptIncomingBiStreams()

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
      logger.warn('MOQtailClient', 'Datagrams already started')
      return
    }

    logger.log('MOQtailClient', 'Starting datagram support...')
    this.#datagramReader = this.webTransport.datagrams.readable.getReader()
    this.#datagramWriter = this.webTransport.datagrams.writable.getWriter()
    this.#isReceivingDatagrams = true

    // Start background datagram reception
    this.#acceptIncomingDatagrams()
    logger.log('MOQtailClient', 'Datagram support started')
  }

  /**
   * Stop receiving datagrams and release resources.
   * Idempotent - safe to call multiple times.
   */
  async stopDatagrams(): Promise<void> {
    if (!this.#isReceivingDatagrams) return

    logger.log('MOQtailClient', 'Stopping datagram support...')
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
    logger.log('MOQtailClient', `Registering datagram handler for trackAlias=${trackAlias}`)
    this.#datagramTrackHandlers.set(key, handler)

    return () => {
      logger.log('MOQtailClient', `Unregistering datagram handler for trackAlias=${trackAlias}`)
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

    logger.log('MOQtailClient', `Creating datagram sender for trackAlias=${trackAlias}`)
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

    const datagram = object.tryIntoDatagram(trackAlias)
    const serialized = datagram.serialize().toUint8Array()
    if (this.onDatagramSent) this.onDatagramSent(datagram)

    await this.#datagramWriter.write(serialized)
  }

  /**
   * Background loop that receives and parses incoming datagrams.
   */
  async #acceptIncomingDatagrams(): Promise<void> {
    logger.log('MOQtailClient', 'Starting datagram reception loop...')

    try {
      while (this.#isReceivingDatagrams && this.#datagramReader) {
        const { done, value: datagramBytes } = await this.#datagramReader.read()

        if (done) {
          logger.log('MOQtailClient', 'Datagram reader done, stopping reception')
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

        try {
          const datagram = Datagram.deserialize(new FrozenByteBuffer(datagramBytes))
          const trackAlias = datagram.trackAlias

          if (this.onDatagramReceived) {
            this.onDatagramReceived(datagram)
          }

          const fullTrackName = this.#resolveTrackAlias(trackAlias)
          const moqtObject = MoqtObject.fromDatagram(datagram, fullTrackName)

          // Dispatch to track-specific handler if registered
          const trackKey = trackAlias.toString()
          const handler = this.#datagramTrackHandlers.get(trackKey)
          if (handler) {
            try {
              handler(moqtObject)
            } catch (handlerError) {
              logger.warn('MOQtailClient', 'Datagram track handler error:', handlerError)
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
          logger.warn('MOQtailClient', 'Failed to parse datagram:', error)
          continue
        }
      }
    } catch (error) {
      logger.error('MOQtailClient', 'Datagram reception error:', error)
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
   * Sets (or replaces) the client-level default early discard policy for incoming subgroup streams.
   *
   * When set, each incoming subgroup QUIC stream is given a deadline of `subgroupReceiveTimeout` ms to
   * complete. If the stream has not finished within that window it is cancelled — objects already
   * delivered to the subscription are kept, but no further objects arrive from that stream.
   *
   * This is a client-wide default. Individual subscriptions can override it via the `earlyDiscardPolicy`
   * field in {@link SubscribeOptions}, which takes precedence over this setting.
   *
   * The policy takes effect on the next stream accepted after this call. Passing a new config
   * replaces the previous one. Pass `undefined` to remove the default.
   *
   * @example
   * ```ts
   * client.setEarlyDiscardPolicy({ subgroupReceiveTimeout: 2000 })
   * ```
   */
  setEarlyDiscardPolicy(config: EarlyDiscardPolicyConfig | undefined): void {
    this.#ensureActive()
    this.#earlyDiscardPolicy = config
  }

  /**
   * Gracefully terminates this {@link MOQtailClient} session and releases underlying {@link https://developer.mozilla.org/docs/Web/API/WebTransport | WebTransport} resources.
   *
   * @param reason - Optional application-level reason (string or error) recorded and wrapped in an {@link InternalError}
   * passed to the {@link MOQtailClient.onSessionTerminated | onSessionTerminated} callback.
   *
   * @returns Promise that resolves once shutdown logic completes. Subsequent calls are safe no-ops.
   *
   * @example Basic usage
   * ```ts
   * await client.disconnect();
   * ```
   *
   * @example With reason
   * ```ts
   * await client.disconnect('user logout');
   * ```
   *
   * @example Idempotent double call
   * ```ts
   * await client.disconnect();
   * await client.disconnect(); // no error
   * ```
   *
   * @example Page unload safety
   * ```ts
   * window.addEventListener('beforeunload', () => {
   *   client.disconnect('page unload');
   * });
   * ```
   */
  async disconnect(reason?: unknown) {
    logger.log('MOQtailClient', 'disconnect', reason)
    if (this.#isDestroyed) return
    this.#isDestroyed = true

    // Stop datagrams first
    await this.stopDatagrams()

    // Close every open request stream so peers see each request cancelled rather than
    // only the session going away.
    const openStreams = [...this.#requestStreams.values()]
    this.#requestStreams.clear()
    this.#namespaceRequestIds.clear()
    await Promise.allSettled(openStreams.map((requestStream) => requestStream.close()))

    if (!this.webTransport.closed) this.webTransport.close()
    if (this.onSessionTerminated)
      this.onSessionTerminated(
        new InternalError('MOQtailClient.disconnect', reason instanceof Error ? reason.message : String(reason)),
      )
  }

  /**
   * Registers or updates a {@link Track} definition for local publishing or serving.
   *
   * A {@link Track} describes a logical media/data stream, identified by a unique name and namespace.
   * - If `trackSource.live` is present, the track can be served to subscribers in real-time.
   * - If `trackSource.past` is present, the track can be fetched for historical data.
   * - If both are present, the track supports both live and historical access.
   *
   * @param track - The {@link Track} instance to add or update. See {@link TrackSource} for live/past source options.
   * @returns void
   * @throws : {@link MOQtailError} If the client has been destroyed.
   *
   * @example Create a live video track from getUserMedia
   * ```ts
   * const stream = await navigator.mediaDevices.getUserMedia({ video: true });
   * const videoTrack = stream.getVideoTracks()[0];
   *
   * // Convert video frames to MoqtObject instances using your chosen scheme (e.g. WARP, CMAF, etc.)
   * // This part is application-specific and not provided by MOQtail:
   * const liveReadableStream: ReadableStream<MoqtObject> = ...
   *
   * // Register the track for live subscription
   * client.addOrUpdateTrack({
   *   fullTrackName: { namespace: ["camera"], name: "main" },
   *   trackSource: { live: liveReadableStream },
   *   publisherPriority: 0 // highest priority
   * });
   *
   * // For a hybrid track (live + past):
   * import { MemoryObjectCache } from './track/object_cache';
   * const cache = new MemoryObjectCache(); // Caches are not yet fully supported
   * client.addOrUpdateTrack({
   *   fullTrackName: { namespace: ["camera"], name: "main" },
   *   trackSource: { live: liveReadableStream, past: cache },
   *   publisherPriority: 8
   * });
   * ```
   */
  addOrUpdateTrack(track: Track) {
    this.#ensureActive()
    if (!isValidTrackAlias(track.trackAlias)) {
      track.trackAlias = random60bitId()
    }
    this.trackSources.set(track.fullTrackName.toString(), track)
  }

  /**
   * Removes a previously registered {@link Track} from this client's local catalog.
   *
   * This deletes the in-memory entry inserted via {@link MOQtailClient.addOrUpdateTrack}, so future lookups by its {@link Track.fullTrackName} will fail.
   * Does **not** automatically:
   * - Send an {@link PublishNamespaceDone} (call {@link MOQtailClient.publishNamespaceDone} separately if you want to inform peers)
   * - Cancel active subscriptions or fetches (they continue until normal completion)
   * - Affect already-sent objects.
   *
   * If the track was not present, the call is a silent no-op (idempotent removal).
   *
   * @param track - The exact {@link Track} instance (its canonical name is used as the key).
   * @throws : {@link MOQtailError} If the client has been destroyed.
   *
   * @example
   * ```ts
   * // Register a track
   * client.addOrUpdateTrack(track);
   *
   * // Later, when no longer publishing:
   * client.removeTrack(track);
   *
   * // Optionally, inform peers that the namespace is no longer available:
   * await client.publishNamespaceDone(track.fullTrackName.namespace);
   * ```
   */
  removeTrack(track: Track) {
    this.#ensureActive()
    this.trackSources.delete(track.fullTrackName.toString())
  }

  /**
   * Subscribes to a track and returns a stream of {@link MoqtObject}s matching the requested window and relay forwarding mode.
   *
   * - `forward: true` tells the relay to forward objects to this subscriber as they arrive.
   * - `forward: false` means the relay subscribes upstream but buffers objects locally, not forwarding them to you.
   * - `filterType: AbsoluteStart` lets you specify a start position in the future; the stream waits for that object. If the start location is \< the latest object
   * observed at the publisher then it behaves as `filterType: LatestObject`
   * - `filterType: AbsoluteRange` lets you specify a start and end group, both of should be in the future; the stream waits for those objects. If the start location is \< the latest object
   * observed at the publisher then it behaves as `filterType: LatestObject`.
   *
   * The method returns either a {@link RequestError} (on refusal) or an object with the subscription `requestId` and a `ReadableStream` of {@link MoqtObject}s.
   * Use the `requestId` for {@link MOQtailClient.unsubscribe} or {@link MOQtailClient.subscribeUpdate}. Use the `stream` to decode and display objects.
   *
   * @param args - {@link SubscribeOptions} describing the subscription window and relay forwarding behavior.
   * @returns Either a {@link RequestError} or `{ requestId, stream }` for consuming objects.
   * @throws : {@link MOQtailError} If the client is destroyed.
   * @throws : {@link ProtocolViolationError} If required fields are missing or inconsistent.
   * @throws : {@link InternalError} On transport/protocol failure (disconnect is triggered before rethrow).
   *
   * @example Subscribe to the latest object and receive future objects as they arrive
   * ```ts
   * const result = await client.subscribe({
   *   fullTrackName,
   *   filterType: FilterType.LatestObject,
   *   forward: true,
   *   groupOrder: GroupOrder.Original,
   *   priority: 32
   * });
   * if (!(result instanceof RequestError)) {
   *   for await (const obj of result.stream) {
   *     // decode and display obj
   *   }
   * }
   * ```
   *
   * @example Subscribe to a future range (waits for those objects to arrive)
   * ```ts
   * const result = await client.subscribe({
   *   fullTrackName,
   *   filterType: FilterType.AbsoluteRange,
   *   startLocation: futureStart,
   *   endGroup: futureEnd,
   *   forward: true,
   *   groupOrder: GroupOrder.Original,
   *   priority: 128
   * });
   * ```
   */
  async subscribe(
    args: SubscribeOptions,
  ): Promise<RequestError | { requestId: bigint; stream: ReadableStream<MoqtObject> }> {
    this.#ensureActive()
    try {
      let { fullTrackName, priority, groupOrder, forward, filterType, parameters, startLocation, endGroup } = args

      logger.debug(
        'MOQtailClient',
        `subscribe: ftn="${fullTrackName}" filterType=${filterType} priority=${priority} forward=${forward} groupOrder=${groupOrder}`,
      )

      let msg: Subscribe
      if (typeof endGroup === 'number') endGroup = BigInt(endGroup)
      const baseParams: MessageParameter[] = [
        new SubscriberPriority(priority),
        new Forward(forward),
        ...(groupOrder !== GroupOrder.Original ? [new GroupOrderParam(groupOrder)] : []),
        ...(parameters ?? []),
      ]
      switch (filterType) {
        case FilterType.LatestObject:
          msg = Subscribe.newLatestObject(this.#nextClientRequestId, fullTrackName, baseParams)
          break
        case FilterType.NextGroupStart:
          msg = Subscribe.newNextGroupStart(this.#nextClientRequestId, fullTrackName, baseParams)
          break
        case FilterType.AbsoluteStart:
          if (!startLocation)
            throw new ProtocolViolationError(
              'MOQtailClient.subscribe',
              'FilterType.AbsoluteStart must have a start location',
            )
          msg = Subscribe.newAbsoluteStart(this.#nextClientRequestId, fullTrackName, startLocation, baseParams)
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
            startLocation,
            endGroup,
            baseParams,
          )
          break
      }
      const request = new SubscribeRequest(msg)
      request.earlyDiscardPolicy = args.earlyDiscardPolicy
      this.requests.set(request.requestId, request)
      this.requestIdMap.addMapping(request.requestId, request.fullTrackName)

      logger.debug('MOQtailClient', `subscribe: sending SUBSCRIBE requestId=${msg.requestId} ftn="${fullTrackName}"`)
      await this.#openRequestStream(request.requestId, msg)
      logger.debug(
        'MOQtailClient',
        `subscribe: SUBSCRIBE sent, awaiting SUBSCRIBE_OK/REQUEST_ERROR requestId=${msg.requestId}`,
      )

      const response = await request

      if (response instanceof RequestError) {
        logger.error(
          'MOQtailClient',
          `subscribe: SUBSCRIBE_ERROR requestId=${request.requestId} code=${response.errorCode} reason="${response.reasonPhrase.phrase}"`,
        )
        this.requests.delete(request.requestId)
        this.requestIdMap.removeMappingByRequestId(request.requestId)
        return response
      } else {
        logger.debug(
          'MOQtailClient',
          `subscribe: SUBSCRIBE_OK requestId=${request.requestId} trackAlias=${response.trackAlias}`,
        )
        this.subscriptions.set(response.trackAlias, request)
        this.subscriptionAliasMap.set(request.requestId, response.trackAlias)
        this.aliasFullTrackNameMap.set(response.trackAlias, fullTrackName)
        return { requestId: msg.requestId, stream: request.stream }
      }
    } catch (error) {
      logger.error(
        'MOQtailClient',
        `subscribe: unexpected error — ${error instanceof Error ? error.message : String(error)}`,
      )
      await this.disconnect(
        new InternalError('MOQtailClient.subscribe', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Stops an active subscription identified by its original SUBSCRIBE `requestId`.
   *
   * Sends an {@link Unsubscribe} control frame if the subscription is still active. If the id is unknown or already
   * cleaned up, the call is a silent no-op (hence multiple calls are idempotent).
   *
   * Use this when you no longer want incoming objects for a track (e.g. user navigated away, switching quality).
   * Canceling the consumer stream reader does **not** auto-unsubscribe; call this explicitly for prompt cleanup.
   *
   * @param requestId - The id returned from {@link MOQtailClient.subscribe}.
   * @returns Promise that resolves when the unsubscribe control frame is sent.
   * @throws :{@link MOQtailError} If the client is destroyed.
   * @throws :{@link InternalError} Wrapped lower-level failure while attempting to send (session will be disconnected first).
   *
   * @remarks
   * - Only targets SUBSCRIBE requests, not fetches. Passing a fetch request id is ignored (no-op).
   * - Safe to call multiple times; extra calls have no effect.
   *
   * @example Subscribe and later unsubscribe
   * ```ts
   * const sub = await client.subscribe({ fullTrackName, filterType: FilterType.LatestObject, forward: true, groupOrder: GroupOrder.Original, priority: 0 });
   * if (!(sub instanceof RequestError)) {
   *   // ...consume objects...
   *   await client.unsubscribe(sub.requestId);
   * }
   * ```
   *
   * @example Idempotent usage
   * ```ts
   * await client.unsubscribe(123n);
   * await client.unsubscribe(123n); // no error
   * ```
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

          // Closing the subscription's request stream is what tells the publisher to stop.
          await this.#closeRequestStream(requestId)
          subscription.unsubscribe()
        }
      }
      // Q: Throw? Idempotent?
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.unsubscribe', error instanceof Error ? error.message : String(error)),
      )
      throw error
    } finally {
      if (cleanupData) {
        this.requests.delete(cleanupData.requestId)
        this.subscriptions.delete(cleanupData.trackAlias)
        this.aliasFullTrackNameMap.delete(cleanupData.trackAlias)
        this.requestIdMap.removeMappingByRequestId(cleanupData.requestId)
      }
    }
  }

  /**
   * Narrows or updates an active subscription window and/or relay forwarding behavior.
   *
   * Use this to:
   * - Move the start of the subscription forward (trim history or future window).
   * - Move the end group earlier (shorten the window).
   * - Change relay forwarding (`forward: false` stops forwarding new objects, `true` resumes).
   * - Adjust subscriber priority.
   *
   * Only narrowing is allowed: you cannot move the start earlier or the end group later than the original subscription.
   * Forwarding and priority can be changed at any time.
   *
   * @param args - {@link SubscribeUpdateOptions} referencing the original subscription `requestId` and new bounds.
   * @returns Promise that resolves when the update control frame is sent.
   * @throws :{@link MOQtailError} If the client is destroyed.
   * @throws :{@link ProtocolViolationError} If the update would widen the window (earlier start, later end group, or invalid ordering).
   * @throws :{@link InternalError} On transport/control failure (disconnect is triggered before rethrow).
   *
   * @remarks
   * - Only applies to active SUBSCRIBE requests; ignored if the request is not a subscription.
   * - Omitting a parameter (e.g. `priority`) leaves the previous value unchanged.
   * - Setting `forward: false` stops relay forwarding new objects after the current window drains.
   * - Safe to call multiple times; extra calls with unchanged bounds have no effect.
   *
   * @example Trim start forward
   * ```ts
   * await client.subscribeUpdate({ requestId, startLocation: laterLoc, endGroup, forward: true, priority });
   * ```
   *
   * @example Convert tailing subscription into bounded slice
   * ```ts
   * await client.subscribeUpdate({ requestId, startLocation: origStart, endGroup: cutoffGroup, forward: false, priority });
   * ```
   *
   * @example Lower priority only
   * ```ts
   * await client.subscribeUpdate({ requestId, startLocation: currentStart, endGroup: currentEnd, forward: true, priority: 200 });
   * ```
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
          if (!isValidTrackAlias(trackAlias))
            throw new InternalError('MOQtailClient.subscribeUpdate', 'Request exists but track alias mapping does not')
          const subscription = this.subscriptions.get(trackAlias)
          if (!subscription)
            throw new InternalError('MOQtailClient.subscribeUpdate', 'Request exists but subscription does not')
          // TODO: If a parameter included in SUBSCRIBE is not present in SUBSCRIBE_UPDATE, its value remains unchanged.
          // There is no mechanism to remove a parameter from a subscription. We can add parameters but check for duplicate params
          const requestId = this.#nextClientRequestId
          const updateParams: MessageParameter[] = [
            new SubscriberPriority(priority),
            new Forward(forward),
            new SubscriptionFilter(FilterType.AbsoluteRange, startLocation, endGroup),
            ...(parameters ?? []),
          ]
          const msg = new RequestUpdate(requestId, updateParams)
          subscription.update(msg) // This also updates the request since both maps store the same object
          // A REQUEST_UPDATE travels on the stream of the request it updates.
          await this.#requestStreamFor(subscriptionRequestId, 'MOQtailClient.subscribeUpdate').send(msg)
        }
      }
      // Q: Throw? Idempotent?
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.subscribeUpdate', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Switches an active subscription to a different track while retaining the same subscription parameters.
   *
   * Use this to change the subscribed track without tearing down and re-establishing a new subscription.
   *
   * @param args - {@link SwitchOptions} referencing the original subscription `requestId` and new track name.
   * @returns Promise that resolves when the switch control frame is sent.
   * @throws :{@link MOQtailError} If the client is destroyed.
   * @throws :{@link InternalError} On transport/control failure (disconnect is triggered before rethrow).
   *
   * @remarks
   * - Only applies to active SUBSCRIBE requests; ignored if the request is not a subscription.
   * - All other subscription parameters (window, forwarding, priority) remain unchanged.
   *
   * @example Switch to a different track
   * ```ts
   * await client.switch({ subscriptionRequestId, fullTrackName: newTrackName });
   * ```
   */
  async switch(args: SwitchOptions): Promise<RequestError | { requestId: bigint; stream: ReadableStream<MoqtObject> }> {
    this.#ensureActive()
    let { fullTrackName, subscriptionRequestId, parameters } = args
    try {
      if (!this.requests.has(subscriptionRequestId))
        throw new ProtocolViolationError('MOQtailClient.switch', 'Unknown subscription request id')

      const request = this.requests.get(subscriptionRequestId)!
      if (!(request instanceof SubscribeRequest))
        throw new ProtocolViolationError('MOQtailClient.switch', 'Request id is not a subscription')

      const trackAlias = this.subscriptionAliasMap.get(subscriptionRequestId)
      if (!isValidTrackAlias(trackAlias))
        throw new InternalError('MOQtailClient.switch', 'Request exists but track alias mapping does not')
      const subscription = this.subscriptions.get(trackAlias)
      if (!subscription) throw new InternalError('MOQtailClient.switch', 'Request exists but subscription does not')

      const requestId = this.#nextClientRequestId
      this.requests.set(requestId, subscription)

      const switchParams: MessageParameter[] = parameters ?? []
      const kvpParams = switchParams.map((p) => p.toKeyValuePair())
      const msg = new Switch(requestId, fullTrackName, subscriptionRequestId, kvpParams)
      subscription.switch(fullTrackName, switchParams)
      // SWITCH retargets an existing subscription, so it goes on that subscription's
      // stream and its SUBSCRIBE_OK comes back there.
      const requestStream = this.#requestStreamFor(subscriptionRequestId, 'MOQtailClient.switch')
      await requestStream.send(msg)
      // The switched subscription is addressed by the new id from here on, so file the
      // stream under it too — unsubscribe(requestId) must still find it.
      this.#requestStreams.set(requestId, requestStream)

      const response = await subscription
      if (response instanceof SubscribeOk) {
        // Generate a new update callback mapping for the new track alias
        this.aliasFullTrackNameMap.set(response.trackAlias, fullTrackName)
        this.pendingStateUpdates.set(subscriptionRequestId, (newTrackAlias: bigint) => {
          if (newTrackAlias !== response.trackAlias) return false
          // Update internal state to expect the new subscription
          this.subscriptions.set(response.trackAlias, subscription)
          this.subscriptionAliasMap.set(requestId, response.trackAlias)
          subscription.requestId = requestId

          // Old subscription id is no longer valid
          this.requestIdMap.removeMappingByRequestId(subscriptionRequestId)
          this.requestIdMap.addMapping(subscriptionRequestId, fullTrackName)

          // remove the old subscription
          this.subscriptions.delete(trackAlias)
          return true
        })

        return { requestId, stream: subscription.stream }
      } else {
        this.requestIdMap.removeMappingByRequestId(requestId)
        this.requests.delete(requestId)
        return response
      }
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.switch', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * One-shot retrieval of a bounded object span, optionally anchored to an existing subscription, returning a stream of {@link MoqtObject}s.
   *
   * Choose a fetch type via `typeAndProps.type`:
   * - Standalone: Historical slice of a specific {@link FullTrackName} independent of active subscriptions.
   * - Relative: Range relative to the JOINING subscription's current (largest) location; use when you want "N groups back" from live.
   * - Absolute: Absolute group/object offsets tied to an existing subscription (stable anchor) even if that subscription keeps forwarding.
   *
   * Field highlights (in {@link FetchOptions}):
   * - priority: 0 (highest) .. 255 (lowest); out-of-range rejected; non-integers rounded by caller expectation.
   * - groupOrder: {@link (GroupOrder:enum).Original} to preserve publisher order; or reorder ascending/descending if supported by server.
   * - typeAndProps: Discriminated union carrying parameters specific to each fetch mode (see examples).
   * - parameters: Optional version-specific extension block.
   *
   * Returns either a {@link RequestError} (refusal / invalid request at protocol level) or `{ requestId, stream }` whose `stream`
   * ends naturally after the bounded range completes (no explicit cancel needed for normal completion).
   *
   * Use cases:
   * - Grab a historical window for scrubbing UI while a separate live subscription tails.
   * - Late joiner fetching a short back-buffer then discarding the stream.
   * - Analytics batch job pulling a fixed slice without subscribing long-term.
   *
   * @throws MOQtailError If client is destroyed.
   * @throws ProtocolViolationError Priority out of [0-255] or missing/invalid joining subscription id for Relative/Absolute.
   * @throws InternalError Transport/control failure (the client disconnects first) then rethrows original error.
   *
   * @remarks
   * - Relative / Absolute require an existing active SUBSCRIBE `joiningRequestId`; if not found a {@link ProtocolViolationError} is thrown.
   * - Result stream is finite; reader close occurs automatically when last object delivered.
   * - Use {@link MOQtailClient.fetchCancel} only for early termination (not yet fully implemented: see TODO in code).
   *
   * @example Standalone window
   * ```ts
   * const r = await client.fetch({
   *   priority: 64,
   *   groupOrder: GroupOrder.Original,
   *   typeAndProps: {
   *     type: FetchType.Standalone,
   *     props: { fullTrackName, startLocation, endLocation }
   *   }
   * })
   * if (!(r instanceof RequestError)) {
   *   for await (const obj of r.stream as any) {
   *     // consume objects then stream ends automatically
   *   }
   * }
   * ```
   *
   * @example Relative to live subscription (e.g. last 5 groups)
   * ```ts
   * const sub = await client.subscribe({ fullTrackName, filterType: FilterType.LatestObject, forward: true, groupOrder: GroupOrder.Original, priority: 0 })
   * if (!(sub instanceof RequestError)) {
   *   const slice = await client.fetch({
   *     priority: 32,
   *     groupOrder: GroupOrder.Original,
   *     typeAndProps: { type: FetchType.Relative, props: { joiningRequestId: sub.requestId, joiningStart: 0n } }
   *   })
   * }
   * ```
   */
  // TODO: figure out how to handle joining fetch types
  // Do we need an existing subscription? What happens if that subscription forwards objects?
  // Will the subscribe objects be pushed through this FetchRequest.controller?
  async fetch(args: FetchOptions): Promise<RequestError | { requestId: bigint; stream: ReadableStream<MoqtObject> }> {
    this.#ensureActive()
    try {
      const { priority, groupOrder, typeAndProps, parameters } = args
      if (priority < 0 || priority > 255)
        throw new ProtocolViolationError(
          'MOQtailClient.fetch',
          `subscriberPriority: ${priority} must be in range of [0-255]`,
        )
      const params: MessageParameter[] = [
        new SubscriberPriority(priority),
        ...(groupOrder !== GroupOrder.Original ? [new GroupOrderParam(groupOrder)] : []),
        ...(parameters ?? []),
      ]
      let msg: Fetch
      let joiningRequest: MOQtailRequest | undefined
      // Generate unique requestId at the beginning to ensure uniqueness
      const requestId = this.#nextClientRequestId
      logger.log(
        'MOQtailClient',
        'fetch: generated requestId:',
        requestId,
        'for fetch type:',
        typeAndProps.type,
        'current #dontUseRequestId:',
        this.#dontUseRequestId,
      )
      switch (typeAndProps.type) {
        case FetchType.Standalone:
          msg = new Fetch(requestId, { type: typeAndProps.type, props: typeAndProps.props }, params)
          break

        case FetchType.Relative:
          joiningRequest = this.requests.get(typeAndProps.props.joiningRequestId)
          if (!(joiningRequest instanceof SubscribeRequest))
            throw new ProtocolViolationError(
              'MOQtailClient.fetch',
              `No subscribe request for the given joiningRequestId: ${typeAndProps.props.joiningRequestId}`,
            )
          msg = new Fetch(requestId, { type: typeAndProps.type, props: typeAndProps.props }, params)
          break
        case FetchType.Absolute:
          joiningRequest = this.requests.get(typeAndProps.props.joiningRequestId)
          if (!(joiningRequest instanceof SubscribeRequest))
            throw new ProtocolViolationError(
              'MOQtailClient.fetch',
              `No subscribe request for the given joiningRequestId: ${typeAndProps.props.joiningRequestId}`,
            )
          msg = new Fetch(requestId, { type: typeAndProps.type, props: typeAndProps.props }, params)
          break
      }
      const request = new FetchRequest(msg)
      logger.log(
        'MOQtailClient',
        'fetch: storing FetchRequest with requestId:',
        msg.requestId,
        'for fetch type:',
        typeAndProps.type,
      )
      logger.log('MOQtailClient', 'fetch: full fetch message:', {
        requestId: msg.requestId,
        fetchType: typeAndProps.type,
        joiningRequestId: typeAndProps.type !== FetchType.Standalone ? typeAndProps.props.joiningRequestId : 'N/A',
      })
      this.requests.set(msg.requestId, request)
      logger.log('MOQtailClient', 'fetch: about to send fetch message to server')
      await this.#openRequestStream(msg.requestId, msg)
      logger.log('MOQtailClient', 'fetch: fetch message sent successfully, waiting for response')
      const response = await request
      if (response instanceof RequestError) {
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
   * Request early termination of an in‑flight FETCH identified by its `requestId`.
   *
   * Use when the consumer no longer needs the remaining objects (user scrubbed away, UI panel closed, replaced by a new fetch).
   * Sends a {@link FetchCancel} control frame if the id currently maps to an active fetch; otherwise silent no-op (idempotent).
   *
   * Parameter semantics:
   * - requestId: bigint returned from {@link MOQtailClient.fetch}. Numbers auto-converted to bigint.
   *
   * Current behavior / limitations:
   * - Data stream closure after cancel is TODO (objects may still arrive briefly).
   * - Unknown / already finished request: ignored without error.
   * - Only targets FETCH requests (not subscriptions).
   *
   * @throws MOQtailError If client is destroyed.
   * @throws InternalError Failure while sending the cancel (client disconnects first).
   *
   * @remarks
   * Follow-up improvement planned: actively close associated readable stream controller immediately upon acknowledgment.
   *
   * @example Cancel shortly after starting
   * ```ts
   * const r = await client.fetch({ priority: 32, groupOrder: GroupOrder.Original, typeAndProps: { type: FetchType.Standalone, props: { fullTrackName, startLocation, endLocation } } })
   * if (!(r instanceof RequestError)) {
   *   // user navigated away
   *   await client.fetchCancel(r.requestId)
   * }
   * ```
   *
   * @example Idempotent double cancel
   * ```ts
   * await client.fetchCancel(456n)
   * await client.fetchCancel(456n) // no error
   * ```
   */
  async fetchCancel(requestId: bigint | number) {
    this.#ensureActive()
    try {
      if (typeof requestId === 'number') requestId = BigInt(requestId)
      const request = this.requests.get(requestId)
      if (request instanceof FetchRequest) {
        // Draft-18 §3.3.2: there is no FETCH_CANCEL. Resetting the fetch's request
        // stream is the cancellation. The FetchRequest stays in `requests` so the
        // objects already in flight still resolve their track name.
        // TODO: mark the fetch's data streams for closure.
        await this.#closeRequestStream(requestId)
      }
      // No matching fetch request, idempotent
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.fetchCancel', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Proactively push a track to the relay/peer.
   */
  async publish(
    fullTrackName: FullTrackName,
    forward: boolean,
    trackAlias: bigint,
    parameters?: MessageParameter[],
    trackExtensions?: TrackExtension[],
  ) {
    this.#ensureActive()
    try {
      const requestId = this.#nextClientRequestId

      const msg = new Publish(
        requestId,
        fullTrackName,
        trackAlias,
        [new Forward(forward), ...(parameters ?? [])],
        trackExtensions ?? [],
      )

      const request = new PublishRequest(msg)

      // Map the alias and request ID for outgoing data multiplexing
      this.requests.set(msg.requestId, request)
      this.requestIdMap.addMapping(msg.requestId, fullTrackName)
      this.subscriptionAliasMap.set(msg.requestId, trackAlias)

      await this.#openRequestStream(msg.requestId, msg)
      const response = await request

      if (response instanceof RequestError) {
        this.requests.delete(msg.requestId)
        this.requestIdMap.removeMappingByRequestId(msg.requestId)
        this.subscriptionAliasMap.delete(msg.requestId)
        return response
      } else {
        // Return the trackAlias so the application can use client.createDatagramSender(trackAlias)
        // or open uni-streams.
        return { requestId: msg.requestId, trackAlias: trackAlias }
      }
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.publish', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Signals the end of a published track to the peer/relay.
   * * @param publishRequestId - The original requestId used when `publish()` was called.
   */
  async publishDone(publishRequestId: bigint | number, statusCode: number = 0, reasonPhrase: string = 'Track Ended') {
    this.#ensureActive()
    try {
      if (typeof publishRequestId === 'number') publishRequestId = BigInt(publishRequestId)

      // Create the PublishDone message. (StreamCount is set to 0n as a default)
      // It carries no request id: the stream it is sent on names the request it ends.
      const msg = new PublishDone(statusCode, 0n, new ReasonPhrase(reasonPhrase))

      // PUBLISH_DONE is the last message on the PUBLISH request's own stream.
      const requestStream = this.#requestStreams.get(publishRequestId)
      if (requestStream) {
        await requestStream.send(msg)
        await this.#closeRequestStream(publishRequestId)
      } else {
        logger.warn('MOQtailClient', `publishDone: no request stream for requestId=${publishRequestId}`)
      }

      // Clean up local publisher-side state
      this.requests.delete(publishRequestId)
      this.requestIdMap.removeMappingByRequestId(publishRequestId)
      this.subscriptionAliasMap.delete(publishRequestId)
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.publishDone', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Registers an incoming PUBLISH announcement as a valid data receiver.
   * This prepares the client to ingest pushed data streams matching the published alias.
   * * @param msg - The incoming Publish control message
   * @returns - A stream of MoqtObjects being pushed by the publisher
   */
  acceptPushedTrack(msg: Publish): ReadableStream<MoqtObject> {
    this.#ensureActive()

    let streamController!: ReadableStreamDefaultController<MoqtObject>
    const stream = new ReadableStream<MoqtObject>({
      start(c) {
        streamController = c
      },
    })

    // 1. Map the request ID to the full track name so the parser knows what track this is
    this.requestIdMap.addMapping(msg.requestId, msg.fullTrackName)

    // 2. Map the request ID to the alias
    this.subscriptionAliasMap.set(msg.requestId, msg.trackAlias)

    this.aliasFullTrackNameMap.set(msg.trackAlias, msg.fullTrackName)

    // 3. Create a pseudo-subscription object that mimics a SubscribeRequest
    // This perfectly matches the shape #handleRecvStreams expects
    const receiver = {
      requestId: msg.requestId,
      streamsAccepted: 0,
      largestLocation: undefined,
      controller: streamController,
    }

    // 4. Register the receiver in the main routing table using the publisher's alias
    this.subscriptions.set(msg.trackAlias, receiver)

    return stream
  }

  // TODO: Each announced track should checked against ongoing subscribe_namespace
  // If matches it should send an announce to that peer automatically
  /**
   * Declare (publish) a track namespace to the peer so subscribers using matching prefixes (via {@link MOQtailClient.subscribeNamespace})
   * can discover and begin subscribing/fetching its tracks.
   *
   * Typical flow (publisher side):
   * 1. Prepare / register one or more {@link Track} objects locally (see {@link MOQtailClient.addOrUpdateTrack}).
   * 2. Call `publishNamespace(namespace)` once per namespace prefix to expose those tracks.
   * 3. Later, call {@link MOQtailClient.publishNamespaceDone} when no longer publishing under that namespace.
   *
   * Parameter semantics:
   * - trackNamespace: Tuple representing the namespace prefix (e.g. ["camera","main"]). All tracks whose full names start with this tuple are considered within the announce scope.
   * - parameters: Optional {@link MessageParameters}; omitted =\> default instance.
   *
   * Returns: {@link RequestOk} on success (namespace added to `announcedNamespaces`) or {@link RequestError} explaining refusal.
   *
   * Use cases:
   * - Make a camera or sensor namespace available before any objects are pushed.
   * - Dynamically expose a newly created room / session namespace.
   * - Re-announce after reconnect to repopulate discovery state.
   *
   * @throws MOQtailError If client is destroyed.
   * @throws InternalError Transport/control failure while sending or awaiting response (client disconnects first).
   *
   * @remarks
   * - Duplicate announce detection is TODO (currently a second call will still send another PUBLISH_NAMESPACE; receiver behavior may vary).
   * - Successful announces are tracked in `announcedNamespaces`; manual removal occurs via {@link MOQtailClient.publishNamespaceDone}.
   * - Discovery subscribers (those who issued {@link MOQtailClient.subscribeNamespace}) will receive the resulting {@link PublishNamespace} message.
   *
   * @example Minimal announce
   * ```ts
   * const res = await client.publishNamespace(["camera","main"])
   * if (res instanceof RequestOk) {
   *   // ready to publish objects under tracks with this namespace prefix
   * }
   * ```
   *
   * @example PublishNamespace with parameters block
   * ```ts
   * const params = new MessageParameters().setSomeExtensionFlag(true)
   * const resp = await client.publishNamespace(["room","1234"], params)
   * ```
   */
  async publishNamespace(trackNamespace: Tuple, parameters?: MessageParameter[]) {
    this.#ensureActive()
    try {
      // TODO: Check for duplicate announces
      const params: MessageParameter[] = parameters ?? []
      const msg = new PublishNamespace(this.#nextClientRequestId, trackNamespace, params)
      const request = new PublishNamespaceRequest(msg.requestId, msg)
      this.requests.set(msg.requestId, request)
      await this.#openRequestStream(msg.requestId, msg)
      const response = await request

      if (response instanceof RequestOk) {
        this.announcedNamespaces.add(msg.trackNamespace)
        // The stream stays open for as long as the namespace is announced; closing it
        // is what withdraws the announcement (see publishNamespaceDone).
        this.#namespaceRequestIds.set(msg.trackNamespace.toUtf8Path(), msg.requestId)
      } else {
        await this.#closeRequestStream(msg.requestId)
      }

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
   * Withdraw a previously announced namespace so new subscribers no longer discover its tracks.
   *
   * Use when shutting down publishing for a logical scope (camera offline, room closed, session ended).
   * Removes the namespace from `announcedNamespaces` locally and sends an {@link PublishNamespaceDone} control frame.
   *
   * Parameter semantics:
   * - trackNamespace: Exact tuple used during {@link MOQtailClient.publishNamespace}. Must match to be removed from internal set.
   *
   * Behavior:
   * - Does not delete locally registered {@link Track} objects (they remain in `trackSources`).
   * - Does not forcibly end active subscriptions that were already established; peers simply stop discovering it for new ones.
   * - Silent if the namespace was not currently recorded (idempotent style).
   *
   * @throws MOQtailError If client is destroyed before sending.
   * @throws (rethrows original error) Any lower-level failure while sending results in a disconnect (unwrapped TODO: future wrap with InternalError for consistency).
   *
   * @remarks
   * Peers that issued {@link MOQtailClient.subscribeNamespace} for a matching prefix should receive the resulting {@link PublishNamespaceDone}.
   * Consider calling this before {@link MOQtailClient.disconnect} to give consumers prompt notice.
   *
   * @example Basic usage
   * ```ts
   * await client.publishNamespaceDone(["camera","main"])
   * ```
   *
   * @example Idempotent
   * ```ts
   * await client.publishNamespaceDone(["camera","main"]) // first time
   * await client.publishNamespaceDone(["camera","main"]) // no error, already removed
   * ```
   */
  async publishNamespaceDone(trackNamespace: Tuple) {
    this.#ensureActive()
    try {
      this.announcedNamespaces.delete(trackNamespace)
      // Draft-18 §3.3.2: there is no PUBLISH_NAMESPACE_DONE. Closing the stream the
      // PUBLISH_NAMESPACE opened withdraws the announcement.
      await this.#closeNamespaceRequestStream(trackNamespace)
    } catch (err) {
      // TODO: Match against error cases
      await this.disconnect()
      throw err
    }
  }

  /**
   * Send an {@link PublishNamespaceCancel} to abort a previously issued ANNOUNCE before (or after) the peer fully processes it.
   *
   * Use when an announce was sent prematurely (e.g. validation failed locally, namespace no longer needed) and you want
   * to retract it without waiting for normal announce lifecycle or before publishing any objects.
   *
   * Parameter semantics:
   * - msg: Pre-constructed {@link PublishNamespaceCancel} referencing the original announce request id / namespace (builder provided elsewhere).
   *
   * Behavior:
   * - Simply forwards the control frame; does not modify `announcedNamespaces` (call {@link MOQtailClient.publishNamespaceDone} for local bookkeeping removal).
   * - Safe to send even if the announce already succeeded; peer may ignore duplicates per spec guidance.
   *
   * @throws MOQtailError If client is destroyed.
   * @throws InternalError Wrapped transport/control send failure (client disconnects first) then rethrows.
   *
   * @remarks
   * Use in tandem with internal tracking if you want to prevent subsequent object publication until a new announce is issued.
   *
   * @example Cancel immediately after a mistaken announce
   * ```ts
   * const publishNamespaceResp = await client.publishNamespace(["camera","temp"]) // wrong namespace
   * // Assume you kept the original announce requestId (e.g. from PublishNamespaceRequest)
   * const cancelMsg = new PublishNamespaceCancel(publishNamespaceResp.requestId as bigint)
   * await client.publishNamespaceCancel(cancelMsg)
   * ```
   */
  async publishNamespaceCancel(msg: PublishNamespaceCancel) {
    this.#ensureActive()
    try {
      // Draft-18 §3.3.2: there is no PUBLISH_NAMESPACE_CANCEL either. Retracting the
      // announce is a reset of the stream it was sent on.
      await this.#closeRequestStream(msg.requestId)
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

  async subscribeNamespace(
    trackNamespacePrefix: Tuple,
    subscribeOptions: NamespaceSubscribeOptions = NamespaceSubscribeOptions.Both,
    parameters?: MessageParameter[],
  ): Promise<{ response: RequestOk | RequestError; cancel: () => Promise<void> }> {
    this.#ensureActive()
    try {
      const params: MessageParameter[] = parameters ?? []
      const msg = new SubscribeNamespace(this.#nextClientRequestId, trackNamespacePrefix, subscribeOptions, params)

      // No #openRequestStream here: NAMESPACE / NAMESPACE_DONE carry only a suffix, so
      // they are only meaningful next to the prefix that opened this stream. The generic
      // pump has no prefix to hand them, hence the bespoke drain below.
      const requestStream = await RequestStream.open(this.webTransport, msg)
      this.#requestStreams.set(msg.requestId, requestStream)

      logger.log(
        'MOQtailClient',
        'subscribeNamespace | sent msg',
        msg,
        msg.trackNamespacePrefix.toUtf8Path(),
        msg.requestId,
      )

      const response = await requestStream.next()

      if (!response) {
        throw new InternalError('MOQtailClient.subscribeNamespace', 'Stream closed before response')
      }
      if (!(response instanceof RequestOk || response instanceof RequestError)) {
        throw new ProtocolViolationError('MOQtailClient.subscribeNamespace', 'Unexpected response message type')
      }

      logger.log('MOQtailClient', 'subscribeNamespace | got response', response)

      if (response instanceof RequestOk) {
        this.subscribedAnnounces.add(trackNamespacePrefix)
        this.#namespaceRequestIds.set(trackNamespacePrefix.toUtf8Path(), msg.requestId)
        void this.#drainNamespaceStream(requestStream, msg.requestId, trackNamespacePrefix)
      } else {
        await this.#closeRequestStream(msg.requestId)
      }

      return {
        response,
        cancel: async () => {
          this.subscribedAnnounces.delete(trackNamespacePrefix)
          this.#namespaceRequestIds.delete(trackNamespacePrefix.toUtf8Path())
          await this.#closeRequestStream(msg.requestId)
        },
      }
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.subscribeNamespace', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  async #drainNamespaceStream(requestStream: RequestStream, requestId: bigint, prefix: Tuple): Promise<void> {
    try {
      while (true) {
        const msg = await requestStream.next()
        if (!msg) break
        if (msg instanceof Namespace && this.onPeerNamespace) {
          this.onPeerNamespace(prefix, msg.trackNamespaceSuffix)
        } else if (msg instanceof NamespaceDone && this.onPeerNamespaceDone) {
          this.onPeerNamespaceDone(prefix, msg.trackNamespaceSuffix)
        }
      }
    } finally {
      this.#requestStreams.delete(requestId)
      this.#namespaceRequestIds.delete(prefix.toUtf8Path())
    }
  }

  async unsubscribeNamespace(msg: UnsubscribeNamespace) {
    this.#ensureActive()
    try {
      // Draft-18 §3.3.2: there is no UNSUBSCRIBE_NAMESPACE. Closing the stream the
      // SUBSCRIBE_NAMESPACE opened ends the prefix subscription.
      this.subscribedAnnounces.delete(msg.trackNamespacePrefix)
      await this.#closeNamespaceRequestStream(msg.trackNamespacePrefix)
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.unsubscribeNamespace', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Asks the peer for the current status of a track.
   *
   * TRACK_STATUS is the seventh `First`-marked type: it opens its own bidi stream and
   * the REQUEST_OK answering it comes back there, after which the stream is closed.
   *
   * @param fullTrackName - The track to report on.
   * @param trackAlias - Alias to carry in the request.
   * @param parameters - Optional message parameters.
   * @returns A {@link RequestOk} on success or a {@link RequestError} on refusal.
   *
   * @example
   * ```ts
   * const status = await client.trackStatus(fullTrackName, trackAlias)
   * if (!(status instanceof RequestError)) {
   *   // track exists
   * }
   * ```
   */
  async trackStatus(
    fullTrackName: FullTrackName,
    trackAlias: bigint,
    parameters?: MessageParameter[],
  ): Promise<RequestOk | RequestError> {
    this.#ensureActive()
    try {
      const msg = TrackStatus.newLatestObject(
        this.#nextClientRequestId,
        trackAlias,
        fullTrackName,
        128,
        GroupOrder.Original,
        false,
        parameters ?? [],
      )
      const request = new TrackStatusRequest(msg)
      this.requests.set(msg.requestId, request)
      await this.#openRequestStream(msg.requestId, msg)
      const response = await request
      await this.#closeRequestStream(msg.requestId)
      this.requests.delete(msg.requestId)
      return response
    } catch (error) {
      await this.disconnect(
        new InternalError('MOQtailClient.trackStatus', error instanceof Error ? error.message : String(error)),
      )
      throw error
    }
  }

  /**
   * Opens a bidi request stream for `first` and starts reading responses off it.
   *
   * @param requestId - The requestId carried by `first`; the key the stream is filed under.
   */
  async #openRequestStream(requestId: bigint, first: ControlMessage): Promise<RequestStream> {
    const requestStream = await RequestStream.open(this.webTransport, first)
    this.#requestStreams.set(requestId, requestStream)
    void this.#pumpRequestStream(requestId, requestStream)
    return requestStream
  }

  /**
   * The stream a previously issued request runs on.
   *
   * @throws :{@link InternalError} If the request has no open stream — it was never
   * issued, or it has already completed or been cancelled.
   */
  #requestStreamFor(requestId: bigint, context: string): RequestStream {
    const requestStream = this.#requestStreams.get(requestId)
    if (!requestStream) throw new InternalError(context, `No open request stream for request id ${requestId}`)
    return requestStream
  }

  /**
   * Reads responses and follow-ups off a locally opened request stream until the peer
   * closes it. Each message belongs to this request by virtue of the stream it arrived
   * on, so no request id is consulted to route it.
   */
  async #pumpRequestStream(requestId: bigint, requestStream: RequestStream): Promise<void> {
    // §10.5: the response is the first message on the response stream, so anything
    // arriving here means the request was answered.
    let answered = false
    try {
      while (true) {
        const msg = await requestStream.next()
        if (!msg) break
        const handler = getHandlerForRequestStreamMessage(msg)
        if (!handler) {
          throw new ProtocolViolationError(
            'MOQtailClient',
            `${msg.constructor.name} is not valid on a request stream (request id ${requestId})`,
          )
        }
        await handler(this, msg, requestStream, requestId)
        answered = true
      }
      logger.debug('MOQtailClient', `request stream for requestId=${requestId} closed by peer`)
    } catch (error) {
      logger.error('MOQtailClient', `request stream for requestId=${requestId} failed`, error)
    } finally {
      this.#requestStreams.delete(requestId)
      // A stream that dies before answering leaves the caller awaiting forever.
      if (!answered) {
        this.requests
          .get(requestId)
          ?.reject(new InternalError('MOQtailClient', `Request stream closed before answering request ${requestId}`))
      }
    }
  }

  /** Closes the request stream filed under `requestId`, if it is still open. */
  async #closeRequestStream(requestId: bigint): Promise<void> {
    const requestStream = this.#requestStreams.get(requestId)
    if (!requestStream) return
    this.#requestStreams.delete(requestId)
    await requestStream.close()
  }

  /** Closes the request stream opened for `namespace`, if there is one. */
  async #closeNamespaceRequestStream(namespace: Tuple): Promise<void> {
    const path = namespace.toUtf8Path()
    const requestId = this.#namespaceRequestIds.get(path)
    if (requestId === undefined) return
    this.#namespaceRequestIds.delete(path)
    await this.#closeRequestStream(requestId)
  }

  /**
   * Reads the shared control stream, which after draft-18 §3.3 carries only SETUP and
   * GOAWAY. SETUP is consumed by the handshake, so GOAWAY is all that is handled here;
   * every request type has its own bidi stream.
   */
  async #handleIncomingControlMessages(): Promise<void> {
    this.#ensureActive()
    try {
      const reader = this.controlStream.stream.getReader()
      while (true) {
        const { done, value: msg } = await reader.read()
        if (done) throw new MOQtailError('WebTransport session is terminated')
        const handler = getHandlerForControlMessage(msg)
        if (!handler) {
          // Strictly a PROTOCOL_VIOLATION, but the relay still pushes a few messages
          // here (PUBLISH_DONE, REQUEST_UPDATE fan-out) that draft-18 puts on request
          // streams. Warn rather than tear the session down until that is cleaned up.
          logger.warn(
            'MOQtailClient',
            `${msg.constructor.name} on the control stream; draft-18 allows only SETUP and GOAWAY there`,
          )
          continue
        }
        await handler(this, msg)
      }
    } catch (error) {
      this.disconnect()
      throw error
    }
  }

  async #acceptIncomingUniStreams() {
    this.#ensureActive()
    const reader = this.#incomingUniStreams
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
        logger.error('MOQtailClient', 'acceptIncomingUniStreams error', error)
        if (this.#isDestroyed) break
      }
    }
  }
  #acceptIncomingBiStreams(): void {
    void (async () => {
      const reader = this.webTransport.incomingBidirectionalStreams.getReader()
      while (true) {
        const { value: biStream, done } = await reader.read()
        if (done) break
        void this.#dispatchIncomingRequestStream(new RequestStream(biStream))
      }
    })()
  }

  /**
   * Serves one peer-opened request stream: the first message must be a `First`-marked
   * type, its handler answers on this same stream, and follow-ups are read until the
   * peer closes it. That close is the peer's cancellation, so it tears down whatever
   * the first message started.
   */
  async #dispatchIncomingRequestStream(requestStream: RequestStream): Promise<void> {
    const first = await requestStream.next()
    if (!first) return

    if (!ControlMessageType.isFirst(first.getType())) {
      logger.warn('MOQtailClient', `${first.constructor.name} may not open a request stream; resetting it`)
      await requestStream.close()
      return
    }

    // The first message names the request; everything later on this stream belongs to it.
    const openingRequestId = (first as { requestId: bigint }).requestId

    try {
      let msg: ControlMessage | undefined = first
      while (msg) {
        const handler = getHandlerForRequestStreamMessage(msg)
        if (!handler) {
          throw new ProtocolViolationError(
            'MOQtailClient',
            `No handler for ${msg.constructor.name} on a request stream`,
          )
        }
        await handler(this, msg, requestStream, openingRequestId)
        msg = await requestStream.next()
      }
    } catch (error) {
      logger.error('MOQtailClient', 'incoming request stream failed', error)
    } finally {
      // The peer closed or reset the stream: cancel whatever it was serving. Every
      // First-marked type carries a request id, but the union as a whole does not.
      const requestId = 'requestId' in first ? first.requestId : undefined
      if (requestId !== undefined) {
        this.publications.get(requestId)?.cancel()
        this.publications.delete(requestId)
      }
      await requestStream.close()
    }
  }

  // TODO: Handle request cancellation. Cancel streams are expected to receive some on-fly objects.
  // Do a timeout? Wait for certain amount of objects?
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
            case FetchType.Standalone:
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
                // Fetch data stream complete - don't delete request here, FetchOk handler will do it
                request.controller?.close()
                break
              }
              if (nextObject) {
                if (nextObject instanceof FetchObject) {
                  if (nextObject.kind === 'end_of_range') {
                    // Draft-16 §10.4.4.2: End-of-Range markers describe gaps in the
                    // response and do not carry application payloads. Skip for now.
                    continue
                  }
                  // TODO: validate if it's a valid fetch object, asc or desc?
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
        let subscription = this.subscriptions.get(header.trackAlias)

        // Check pending state updates for switch operations
        if (!subscription) {
          for (const [subscriptionId, callback] of this.pendingStateUpdates) {
            const matched = callback(header.trackAlias)
            if (matched) {
              subscription = this.subscriptions.get(header.trackAlias)
              this.pendingStateUpdates.delete(subscriptionId)
              break
            }
          }
        }

        if (subscription) {
          subscription.streamsAccepted++
          let firstObjectId: bigint | null = null

          let subgroupTimeoutId: ReturnType<typeof setTimeout> | undefined
          const effectiveDiscardPolicy = subscription.earlyDiscardPolicy ?? this.#earlyDiscardPolicy
          if (effectiveDiscardPolicy?.subgroupReceiveTimeout !== undefined) {
            subgroupTimeoutId = setTimeout(() => {
              reader.cancel('early discard: subgroupReceiveTimeout exceeded').catch(() => {})
            }, effectiveDiscardPolicy.subgroupReceiveTimeout)
          }

          try {
            while (true) {
              const { done, value: nextObject } = await reader.read()
              if (done) {
                break
              }
              if (nextObject) {
                if (nextObject instanceof SubgroupObject) {
                  // TODO: validate if it's a valid subgroup object
                  if (!firstObjectId) {
                    firstObjectId = nextObject.objectId
                  }
                  let subgroupId: bigint | null = null
                  if (SubgroupHeaderType.isSubgroupIdZero(header.type)) {
                    subgroupId = 0n
                  } else if (SubgroupHeaderType.isSubgroupIdFirstObjectId(header.type)) {
                    subgroupId = firstObjectId ?? null
                  } else if (SubgroupHeaderType.hasExplicitSubgroupId(header.type)) {
                    subgroupId = header.subgroupId ?? null
                  }

                  const fullTrackName = this.aliasFullTrackNameMap.get(header.trackAlias)
                  if (!fullTrackName) {
                    throw new ProtocolViolationError('MOQtailClient', 'No full track name for received track alias')
                  }

                  const moqtObject = MoqtObject.fromSubgroupObject(
                    nextObject,
                    header.groupId,
                    header.publisherPriority,
                    subgroupId,
                    fullTrackName,
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
          } finally {
            if (subgroupTimeoutId !== undefined) clearTimeout(subgroupTimeoutId)
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
      //this.disconnect()
      throw error
    }
  }
}

if (import.meta.vitest) {
  const { describe, it, expect, afterEach, vi } = import.meta.vitest

  /** One bidirectional stream: what the client wrote, and a way to answer on it. */
  class MockBidiStream {
    readonly sentChunks: Uint8Array[] = []
    readonly readable: ReadableStream<Uint8Array>
    readonly writable: WritableStream<Uint8Array>
    isClosed = false
    #peer!: ReadableStreamDefaultController<Uint8Array>

    constructor() {
      this.readable = new ReadableStream<Uint8Array>({
        start: (controller) => {
          this.#peer = controller
        },
      })
      this.writable = new WritableStream<Uint8Array>({
        write: (chunk) => {
          this.sentChunks.push(chunk)
        },
        close: () => {
          this.isClosed = true
        },
        abort: () => {
          this.isClosed = true
        },
      })
    }

    /** Delivers `msg` to the client on this stream, as the peer would. */
    respond(msg: ControlMessage): void {
      this.#peer.enqueue(ControlMessage.serialize(msg).toUint8Array())
    }

    /** Everything the client has written to this stream so far. */
    get messages(): ControlMessage[] {
      return this.sentChunks.map((chunk) => ControlMessage.deserialize(new FrozenByteBuffer(chunk)))
    }
  }

  class MockWebTransport {
    static last: MockWebTransport
    readonly ready = Promise.resolve()
    readonly closed = new Promise<void>(() => {})
    readonly sentChunks: Uint8Array[] = []
    readonly uniStreamOptions: unknown[] = []
    /** Bidi streams the client opened, in order. */
    readonly biStreams: MockBidiStream[] = []
    readonly incomingBidirectionalStreams: ReadableStream<WebTransportBidirectionalStream>
    readonly incomingUnidirectionalStreams: ReadableStream<ReadableStream<Uint8Array>>
    #incoming!: ReadableStreamDefaultController<ReadableStream<Uint8Array>>
    #incomingBi!: ReadableStreamDefaultController<WebTransportBidirectionalStream>

    async createBidirectionalStream(): Promise<WebTransportBidirectionalStream> {
      const biStream = new MockBidiStream()
      this.biStreams.push(biStream)
      return biStream as unknown as WebTransportBidirectionalStream
    }

    /** Opens a peer-initiated bidi stream, as the relay would for a forwarded request. */
    openIncomingBiStream(): MockBidiStream {
      const biStream = new MockBidiStream()
      this.#incomingBi.enqueue(biStream as unknown as WebTransportBidirectionalStream)
      return biStream
    }

    constructor() {
      this.incomingBidirectionalStreams = new ReadableStream<WebTransportBidirectionalStream>({
        start: (controller) => {
          this.#incomingBi = controller
        },
      })
      this.incomingUnidirectionalStreams = new ReadableStream<ReadableStream<Uint8Array>>({
        start: (controller) => {
          this.#incoming = controller
        },
      })
      MockWebTransport.last = this
    }

    async createUnidirectionalStream(options?: unknown): Promise<WritableStream<Uint8Array>> {
      this.uniStreamOptions.push(options)
      return new WritableStream<Uint8Array>({
        write: (chunk) => {
          this.sentChunks.push(chunk)
        },
      })
    }

    /** Opens a peer uni stream carrying `bytes`, left open so the reader keeps waiting. */
    openIncomingUniStream(bytes: Uint8Array): void {
      this.#incoming.enqueue(
        new ReadableStream<Uint8Array>({
          start: (controller) => controller.enqueue(bytes),
        }),
      )
    }

    close(): void {}
  }

  describe('MOQtailClient control plane', () => {
    const originalWebTransport = globalThis.WebTransport

    afterEach(() => {
      globalThis.WebTransport = originalWebTransport
    })

    function connect(): Promise<MOQtailClient> {
      globalThis.WebTransport = MockWebTransport as unknown as typeof WebTransport
      return MOQtailClient.new({ url: 'https://relay.example/moq' })
    }

    it('handshakes over a pair of uni streams, SETUP first in both directions', async () => {
      const connecting = connect()
      const transport = MockWebTransport.last
      await vi.waitFor(() => expect(transport.sentChunks).toHaveLength(1))

      transport.openIncomingUniStream(new Setup(new SetupOptions().build()).serialize().toUint8Array())
      const client = await connecting

      expect(transport.uniStreamOptions).toEqual([{ sendOrder: Number.MAX_SAFE_INTEGER }])
      expect(ControlMessage.deserialize(new FrozenByteBuffer(transport.sentChunks[0]!))).toBeInstanceOf(Setup)
      await client.disconnect()
    })

    it('rejects a peer control stream that does not begin with SETUP', async () => {
      const connecting = connect()
      const transport = MockWebTransport.last
      await vi.waitFor(() => expect(transport.sentChunks).toHaveLength(1))

      transport.openIncomingUniStream(new GoAway('https://elsewhere.example').serialize().toUint8Array())
      await expect(connecting).rejects.toThrow('Expected setup as the first control message')
    })
  })

  describe('MOQtailClient request streams', () => {
    const originalWebTransport = globalThis.WebTransport
    const ftn = FullTrackName.tryNew('room/alice', 'video')

    afterEach(() => {
      globalThis.WebTransport = originalWebTransport
    })

    /** A connected client whose handshake is already done. */
    async function connected(): Promise<{ client: MOQtailClient; transport: MockWebTransport }> {
      globalThis.WebTransport = MockWebTransport as unknown as typeof WebTransport
      const connecting = MOQtailClient.new({ url: 'https://relay.example/moq' })
      const transport = MockWebTransport.last
      await vi.waitFor(() => expect(transport.sentChunks).toHaveLength(1))
      transport.openIncomingUniStream(new Setup(new SetupOptions().build()).serialize().toUint8Array())
      return { client: await connecting, transport }
    }

    /** Waits for the client to open its n-th bidi stream and write its first message. */
    async function openedStream(transport: MockWebTransport, index: number): Promise<MockBidiStream> {
      await vi.waitFor(() => {
        expect(transport.biStreams.length).toBeGreaterThan(index)
        expect(transport.biStreams[index]!.sentChunks.length).toBeGreaterThan(0)
      })
      return transport.biStreams[index]!
    }

    it('opens a stream per request and answers each on the stream it came from', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        filterType: FilterType.LatestObject,
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const subscribeStream = await openedStream(transport, 0)
      const subscribeMsg = subscribeStream.messages[0]
      expect(subscribeMsg).toBeInstanceOf(Subscribe)
      subscribeStream.respond(SubscribeOk.create((subscribeMsg as Subscribe).requestId, 7n, [], []))
      expect(await subscribing).toMatchObject({ requestId: (subscribeMsg as Subscribe).requestId })

      const fetching = client.fetch({
        priority: 0,
        groupOrder: GroupOrder.Original,
        typeAndProps: {
          type: FetchType.Standalone,
          props: { fullTrackName: ftn, startLocation: new Location(0n, 0n), endLocation: new Location(1n, 0n) },
        },
      })
      const fetchStream = await openedStream(transport, 1)
      const fetchMsg = fetchStream.messages[0]
      expect(fetchMsg).toBeInstanceOf(Fetch)
      fetchStream.respond(new FetchOk((fetchMsg as Fetch).requestId, false, new Location(1n, 0n), []))
      expect(await fetching).toMatchObject({ requestId: (fetchMsg as Fetch).requestId })

      const publishing = client.publish(ftn, true, 9n)
      const publishStream = await openedStream(transport, 2)
      const publishMsg = publishStream.messages[0]
      expect(publishMsg).toBeInstanceOf(Publish)
      // PUBLISH is answered by REQUEST_OK; PUBLISH_OK is only that message's alias here.
      publishStream.respond(new RequestOk((publishMsg as Publish).requestId))
      expect(await publishing).toMatchObject({ trackAlias: 9n })

      const announcing = client.publishNamespace(Tuple.fromUtf8Path('room/alice'))
      const announceStream = await openedStream(transport, 3)
      const announceMsg = announceStream.messages[0]
      expect(announceMsg).toBeInstanceOf(PublishNamespace)
      announceStream.respond(new RequestOk((announceMsg as PublishNamespace).requestId))
      expect(await announcing).toBeInstanceOf(RequestOk)

      const subscribingNs = client.subscribeNamespace(Tuple.fromUtf8Path('room'))
      const subscribeNsStream = await openedStream(transport, 4)
      const subscribeNsMsg = subscribeNsStream.messages[0]
      expect(subscribeNsMsg).toBeInstanceOf(SubscribeNamespace)
      subscribeNsStream.respond(new RequestOk((subscribeNsMsg as SubscribeNamespace).requestId))
      expect((await subscribingNs).response).toBeInstanceOf(RequestOk)

      const statusing = client.trackStatus(ftn, 11n)
      const statusStream = await openedStream(transport, 5)
      const statusMsg = statusStream.messages[0]
      expect(statusMsg).toBeInstanceOf(TrackStatus)
      statusStream.respond(new RequestOk((statusMsg as TrackStatus).requestId))
      expect(await statusing).toBeInstanceOf(RequestOk)

      // Six streams, six requests, and the control stream still carries only the SETUP
      // written during the handshake. SUBSCRIBE_TRACKS is the seventh type; it has no
      // message body yet (#266).
      expect(transport.biStreams).toHaveLength(6)
      expect(transport.sentChunks).toHaveLength(1)
      expect(ControlMessage.deserialize(new FrozenByteBuffer(transport.sentChunks[0]!))).toBeInstanceOf(Setup)

      await client.disconnect()
    })

    it('sends REQUEST_UPDATE on the subscription stream and cancels by closing it', async () => {
      const { client, transport } = await connected()

      const subscribing = client.subscribe({
        fullTrackName: ftn,
        filterType: FilterType.LatestObject,
        forward: true,
        groupOrder: GroupOrder.Original,
        priority: 0,
      })
      const subscribeStream = await openedStream(transport, 0)
      const requestId = (subscribeStream.messages[0] as Subscribe).requestId
      subscribeStream.respond(SubscribeOk.create(requestId, 7n, [], []))
      await subscribing

      await client.subscribeUpdate({
        subscriptionRequestId: requestId,
        startLocation: new Location(1n, 0n),
        endGroup: 5n,
        forward: false,
        priority: 200,
      })
      expect(subscribeStream.messages[1]).toBeInstanceOf(RequestUpdate)
      expect(transport.biStreams).toHaveLength(1)

      await client.unsubscribe(requestId)
      // No UNSUBSCRIBE message: closing the stream is the cancellation.
      expect(subscribeStream.messages).toHaveLength(2)
      expect(subscribeStream.isClosed).toBe(true)

      await client.disconnect()
    })

    it('refuses a REQUEST_OK carrying Track Properties outside a TRACK_STATUS_OK', async () => {
      const { client, transport } = await connected()

      const announcing = client.publishNamespace(Tuple.fromUtf8Path('room/alice'))
      const announceStream = await openedStream(transport, 0)
      const announceMsg = announceStream.messages[0] as PublishNamespace
      // §10.5 populates Track Properties in a TRACK_STATUS_OK and nowhere else, so this
      // PUBLISH_NAMESPACE_OK is a protocol violation: the request fails rather than
      // resolving with a namespace the peer never really accepted.
      announceStream.respond(new RequestOk(announceMsg.requestId, [], [new DeliveryTimeoutExtension(5000n)]))

      // Without the properties the same exchange resolves; see the request-per-stream
      // test above.
      await expect(announcing).rejects.toThrow()
    })

    it('answers a peer-opened request stream on that stream', async () => {
      const { client, transport } = await connected()

      const incoming = transport.openIncomingBiStream()
      const subscribe = Subscribe.newLatestObject(4n, ftn, [])
      incoming.respond(subscribe)

      // No such track is registered, so the refusal comes back here rather than on the
      // control stream.
      await vi.waitFor(() => expect(incoming.messages).toHaveLength(1))
      expect(incoming.messages[0]).toBeInstanceOf(RequestError)
      expect(transport.sentChunks).toHaveLength(1)

      await client.disconnect()
    })

    it('refuses a peer-opened stream that does not begin with a First-marked type', async () => {
      const { client, transport } = await connected()

      const incoming = transport.openIncomingBiStream()
      incoming.respond(new RequestOk(4n))

      await vi.waitFor(() => expect(incoming.isClosed).toBe(true))
      expect(incoming.messages).toHaveLength(0)

      await client.disconnect()
    })
  })
}
