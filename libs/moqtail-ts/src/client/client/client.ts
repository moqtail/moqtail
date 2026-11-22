/**
 * Copyright 2026 The MOQtail Authors
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

import type { ControlStream } from '@/client/control_stream'
import type { SendDatagramStream } from '@/client/datagram_stream'
import type { DataHeaderEvent } from '@/client/data_event'
import type { FetchPublication } from '@/client/publication/fetch'
import type { PublishPublication } from '@/client/publication/publish'
import type { SubscribePublication } from '@/client/publication/subscribe'
import type { SubscribeRequest } from '@/client/request/subscribe'
import type { Track } from '@/client/track/track'
import {
  DEFAULT_FILL_OPEN_TIMEOUT_MS,
  DEFAULT_SWITCH_DRAIN_TIMEOUT_MS,
  DEFAULT_TRACK_ALIAS_RESOLUTION_TIMEOUT_MS,
} from '@/client/types'
import type {
  ClientEvents,
  EarlyDiscardPolicyConfig,
  FetchOptions,
  FetchResult,
  MOQtailClientOptions,
  MOQtailRequest,
  SubscribeOptions,
  SubscribeResult,
  SubscribeUpdateOptions,
  SwitchOptions,
} from '@/client/types'
import type { Tuple } from '@/model/common/tuple'
import { PublishDoneStatusCode } from '@/model/constants'
import type { InternalError, MOQtailError, ProtocolViolationError } from '@/model/error/error'
import type { GoAway, Publish, PublishNamespace, RequestError, RequestOk, Setup } from '@/model/messages'
import type { FullTrackName } from '@/model/data/full_track_name'
import type { MoqtObject } from '@/model/data/object'
import { RequestIdMap } from '@/model/data/request_id'
import type { MessageParameters } from '@/model/parameters'
import type { TrackProperty } from '@/model/property/track_property'
import { Emitter } from '@/util/emitter'
import { setLogEnabledModules, setLogLevel } from '@/util/logger'
import type { LogLevel } from '@/util/logger'
import { fetch, fetchCancel } from './fetch'
import { subscribeNamespace, subscribeTracks, trackStatus, unsubscribeNamespace } from './namespace'
import {
  acceptPushedTrack,
  addOrUpdateTrack,
  publish,
  publishDone,
  publishNamespace,
  removeTrack,
  unpublishNamespace,
} from './publish'
import { migrateRequest } from './request_streams'
import { Session, closeSession, openSession } from './session'
import { completeSubscription, subscribe, subscribeUpdate, switchTrack, unsubscribe } from './subscribe'

/**
 * @public
 * Represents a Media Over QUIC Transport (MOQT) client session.
 *
 * Construct it, attach listeners with `on()`, then call {@link MOQtailClient.init} to open the
 * session. Once initialized, the client provides high-level methods for MOQT requests and publishing. If a protocol
 * violation occurs, the client terminates; a new client is needed to reconnect.
 *
 * ## Usage
 *
 * ### Connect and Subscribe to a Track
 * ```ts
 * const client = new MOQtailClient({ url });
 * client.on('sessionterminated', (reason) => console.warn('Session ended:', reason));
 * await client.init();
 * const result = await client.subscribe({
 *   fullTrackName,
 *   locationFilter: locationFilter(LocationFilterType.NextObject),
 *   forward: true,
 *   groupOrder: GroupOrder.Original,
 *   priority: 0
 * });
 * if (!isRequestError(result)) {
 *   for await (const object of result.stream) {
 *     // Consume MOQT objects
 *   }
 * }
 * ```
 *
 * ### Publish a namespace for Publishing
 * ```ts
 * const client = new MOQtailClient({ url });
 * await client.init();
 * const publishNamespaceResult = await client.publishNamespace(Tuple.fromUtf8Path('camera/main'));
 * if (!isRequestError(publishNamespaceResult)) {
 *   // Ready to publish objects under this namespace
 * }
 * ```
 *
 * ### Graceful Shutdown
 * ```ts
 * await client.disconnect();
 * ```
 */
export class MOQtailClient extends Emitter<ClientEvents> {
  /**
   * Namespace prefixes (tuples) the peer has requested announce notifications for via SUBSCRIBE_NAMESPACE.
   * Used to decide which locally issued ANNOUNCE messages should be forwarded (future optimization: prefix trie).
   */
  readonly peerSubscribeNamespace = new Set<Tuple>()
  /**
   * Namespace prefixes this client has subscribed to (issued SUBSCRIBE_NAMESPACE). Enables automatic filtering
   * of incoming NAMESPACE / NAMESPACE_DONE. Maintained locally; no dedupe of overlapping / shadowing prefixes yet.
   */
  readonly subscribedNamespaces = new Set<Tuple>()
  readonly subscribedTracks = new Set<Tuple>()
  /**
   * Track namespaces this client has successfully published (received REQUEST_OK). Source of truth for
   * deciding what to withdraw on teardown or targeted removal (future optimization: prefix trie).
   */
  readonly publishedNamespaces = new Set<Tuple>()
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
  readonly #session: Session

  /** Underlying WebTransport session (set by {@link MOQtailClient.init}). */
  webTransport!: WebTransport
  /** Outgoing / incoming control message stream pair. */
  controlStream!: ControlStream
  /** Timeout (ms) applied to reading incoming data streams; undefined =\> no explicit timeout. */
  dataStreamTimeoutMs?: number
  /** Timeout (ms) for control stream read operations; undefined =\> no explicit timeout. */
  controlStreamTimeoutMs?: number
  /**
   * How long (ms) an incoming data stream waits for the control message that establishes
   * its track alias before the stream is abandoned.
   */
  trackAliasResolutionTimeoutMs: number = DEFAULT_TRACK_ALIAS_RESOLUTION_TIMEOUT_MS
  /**
   * How long (ms) a track a soft switch handed over from may stay quiet before it is
   * retired anyway, if no PUBLISH_DONE ends its drain first.
   */
  switchDrainTimeoutMs: number = DEFAULT_SWITCH_DRAIN_TIMEOUT_MS
  /** How long (ms) after SUBSCRIBE_OK a fill fetch stream may take to open before ordering stops waiting for it. */
  fillOpenTimeoutMs: number = DEFAULT_FILL_OPEN_TIMEOUT_MS
  /**
   * Whether the peer has sent a GOAWAY on the control stream. Once it has, this side
   * should not start new requests; a second one closes the session with
   * PROTOCOL_VIOLATION.
   */
  goawayReceived = false

  /**
   * Stores `options`; nothing happens on the network until {@link MOQtailClient.init}, so
   * listeners attached in between see everything, the handshake included.
   */
  constructor(options: MOQtailClientOptions) {
    super()
    this.#session = new Session(this, options)
  }

  /**
   * Gets the current server setup configuration.
   *
   * @returns The {@link Setup} instance the server sent back during handshake.
   */
  get serverSetup(): Setup {
    return this.#session.serverSetup
  }

  /**
   * Gets the setup this client sent during handshake.
   *
   * @returns The {@link Setup} instance written first on the control stream.
   */
  get clientSetup(): Setup {
    return this.#session.clientSetup
  }

  /**
   * How many filter ranges this client allows the server in one request: the
   * MAX_FILTER_RANGES it advertised, {@link DEFAULT_MAX_FILTER_RANGES} unless the
   * caller's setup options set one.
   */
  get maxFilterRanges(): bigint {
    return this.#session.maxFilterRanges
  }

  /**
   * How many filter ranges the server allows in one request: its MAX_FILTER_RANGES,
   * or 0 when it sent none, which allows no Range Filter at all.
   */
  get peerMaxFilterRanges(): bigint {
    return this.#session.serverSetup?.options.maxFilterRanges ?? 0n
  }

  /**
   * Returns true if datagram support is currently active
   */
  get isDatagramsEnabled(): boolean {
    return this.#session.datagrams.receiving
  }

  /**
   * Stream of all received MoqtObjects from datagrams across all tracks
   * Consumer should filter by fullTrackName as needed
   *
   * WARNING: Only one reader should be active. For multiple subscribers,
   * use subscribeToTrackDatagrams() instead
   */
  get receivedDatagramObjects(): ReadableStream<MoqtObject> {
    return this.#session.datagrams.objects
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
   * Opens the WebTransport session and performs the MOQT setup handshake. Runs once: a
   * second call throws, and a failed one leaves the client unusable, so a retry needs a
   * new client.
   *
   * @throws :{@link MOQtailError} If called more than once, or after {@link MOQtailClient.disconnect}.
   * @throws :{@link ProtocolViolationError} If the server sends an unexpected or invalid message during setup.
   *
   * @example
   * ```ts
   * const client = new MOQtailClient({ url: 'moqt://relay.moqtail.dev' })
   * client.on('messagesent', (msg) => console.log('Sent:', msg))
   * await client.init()
   * ```
   */
  init(): Promise<void> {
    return openSession(this.#session)
  }

  /**
   * Start receiving datagrams from the WebTransport connection.
   * Must be called before datagrams can be received (unless enableDatagrams: true was set in options).
   *
   * @throws MOQtailError if client is destroyed or datagrams already started
   */
  startDatagrams(): Promise<void> {
    return this.#session.datagrams.start()
  }

  /**
   * Stop receiving datagrams and release resources.
   * Idempotent - safe to call multiple times.
   */
  stopDatagrams(): Promise<void> {
    return this.#session.datagrams.stop()
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
    return this.#session.datagrams.onTrack(trackAlias, handler)
  }

  /**
   * Unsubscribe from datagram delivery for a specific track.
   *
   * @param trackAlias - Track alias to unsubscribe from
   */
  unsubscribeFromTrackDatagrams(trackAlias: bigint): void {
    this.#session.datagrams.offTrack(trackAlias)
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
    return this.#session.datagrams.sender(trackAlias)
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
  sendDatagram(trackAlias: bigint, object: MoqtObject): Promise<void> {
    return this.#session.datagrams.send(trackAlias, object)
  }

  /**
   * The track a data stream carries. A subgroup stream can open before the response
   * that names its alias, so its header event may lack `fullTrackName`: ask again on a
   * later event of the same stream.
   */
  trackNameOf(header: DataHeaderEvent): FullTrackName | undefined {
    if (header.fullTrackName || header.trackAlias === undefined) return header.fullTrackName

    return this.aliasFullTrackNameMap.get(header.trackAlias)
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
    this.#session.ensureActive()
    this.#session.earlyDiscardPolicy = config
  }

  /**
   * Gracefully terminates this {@link MOQtailClient} session and releases underlying {@link https://developer.mozilla.org/docs/Web/API/WebTransport | WebTransport} resources.
   *
   * @param reason - Optional application-level reason (string or error) recorded and wrapped in an {@link InternalError}
   * passed to `sessionterminated` listeners. Before {@link MOQtailClient.init} it only marks the client destroyed.
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
  disconnect(reason?: unknown): Promise<void> {
    return closeSession(this.#session, reason)
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
   * // Convert video frames to MoqtObject instances using your chosen scheme (e.g. MSF, CMAF, etc.)
   * // This part is application-specific and not provided by MOQtail:
   * const liveReadableStream: ReadableStream<MoqtObject> = ...
   *
   * // Register the track for live subscription
   * client.addOrUpdateTrack({
   *   fullTrackName: FullTrackName.tryNew('camera', 'main'),
   *   trackSource: { live: new LiveTrackSource(liveReadableStream) },
   *   publisherPriority: 0 // highest priority
   * });
   *
   * // For a hybrid track (live + past), add each object to the cache as you write it:
   * const cache = new RingBufferObjectCache(300);
   * client.addOrUpdateTrack({
   *   fullTrackName: FullTrackName.tryNew('camera', 'main'),
   *   trackSource: new HybridTrackSource(cache, liveReadableStream),
   *   publisherPriority: 8
   * });
   * ```
   */
  addOrUpdateTrack(track: Track): void {
    addOrUpdateTrack(this.#session, track)
  }

  /**
   * Removes a previously registered {@link Track} from this client's local catalog.
   *
   * This deletes the in-memory entry inserted via {@link MOQtailClient.addOrUpdateTrack}, so future lookups by its {@link Track.fullTrackName} will fail.
   * Does **not** automatically:
   * - Withdraw the namespace announcement (call {@link MOQtailClient.unpublishNamespace} separately if you want to inform peers)
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
   * await client.unpublishNamespace(track.fullTrackName.namespace);
   * ```
   */
  removeTrack(track: Track): void {
    removeTrack(this.#session, track)
  }

  /**
   * Subscribes to a track and returns a stream of {@link MoqtObject}s matching the requested window and relay forwarding mode.
   *
   * - `forward: true` tells the relay to forward objects to this subscriber as they arrive.
   * - `forward: false` means the relay subscribes upstream but buffers objects locally, not forwarding them to you.
   * - `locationFilter` selects which Objects are delivered; omitted, the subscription is unfiltered. An absolute start in the
   * future waits for that object.
   * - A `fillParameters` in `parameters` also retrieves Objects already published, on a fill fetch stream.
   *
   * The method returns either a {@link RequestError} (on refusal) or a {@link SubscribeResult}: the subscription `requestId`, its
   * SUBSCRIBE_OK, a `ReadableStream` of {@link MoqtObject}s, and a `done` promise carrying the PUBLISH_DONE that ends it.
   * Use the `requestId` for {@link MOQtailClient.unsubscribe} or {@link MOQtailClient.subscribeUpdate}. Use the `stream` to decode and display objects.
   *
   * @param args - {@link SubscribeOptions} describing the subscription window and relay forwarding behavior.
   * @returns Either a {@link RequestError} or `{ requestId, subscribeOk, stream, done }`.
   * @throws : {@link MOQtailError} If the client is destroyed.
   * @throws : {@link ProtocolViolationError} If required fields are missing or inconsistent.
   * @throws : `InvalidFilterError` If the Range Filters exceed the server's MAX_FILTER_RANGES; nothing is sent.
   * @throws : {@link InternalError} On transport/protocol failure (disconnect is triggered before rethrow).
   *
   * @example Subscribe to the latest object and receive future objects as they arrive
   * ```ts
   * const result = await client.subscribe({
   *   fullTrackName,
   *   locationFilter: locationFilter(LocationFilterType.NextObject),
   *   forward: true,
   *   groupOrder: GroupOrder.Original,
   *   priority: 32
   * });
   * if (!isRequestError(result)) {
   *   for await (const obj of result.stream) {
   *     // decode and display obj
   *   }
   * }
   * ```
   *
   * @example Subscribe to a future range of groups (waits for those objects to arrive)
   * ```ts
   * const result = await client.subscribe({
   *   fullTrackName,
   *   locationFilter: locationFilter(LocationFilterType.AbsoluteStartGroupEnd, startGroup, 0n, endGroup),
   *   forward: true,
   *   groupOrder: GroupOrder.Original,
   *   priority: 128
   * });
   * ```
   */
  subscribe(args: SubscribeOptions): Promise<RequestError | SubscribeResult> {
    return subscribe(this.#session, args)
  }

  /**
   * Stops an active subscription identified by its original SUBSCRIBE `requestId`.
   *
   * Resets the subscription's request stream with `CANCELLED` if it is still active. If the id is unknown or already
   * cleaned up, the call is a silent no-op (hence multiple calls are idempotent).
   *
   * Use this when you no longer want incoming objects for a track (e.g. user navigated away, switching quality).
   * Canceling the consumer stream reader does **not** auto-unsubscribe; call this explicitly for prompt cleanup.
   *
   * @param requestId - The id returned from {@link MOQtailClient.subscribe}.
   * @returns Promise that resolves once the request stream has been reset.
   * @throws :{@link MOQtailError} If the client is destroyed.
   * @throws :{@link InternalError} Wrapped lower-level failure while attempting to send (session will be disconnected first).
   *
   * @remarks
   * - Only targets SUBSCRIBE requests, not fetches. Passing a fetch request id is ignored (no-op).
   * - Safe to call multiple times; extra calls have no effect.
   *
   * @example Subscribe and later unsubscribe
   * ```ts
   * const sub = await client.subscribe({ fullTrackName, locationFilter: locationFilter(LocationFilterType.NextObject), forward: true, groupOrder: GroupOrder.Original, priority: 0 });
   * if (!isRequestError(sub)) {
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
  unsubscribe(requestId: bigint | number): Promise<void> {
    return unsubscribe(this.#session, BigInt(requestId))
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
   * await client.subscribeUpdate({ subscriptionRequestId, locationFilter: locationFilter(LocationFilterType.AbsoluteStart, laterGroup, 0n), forward: true, priority });
   * ```
   *
   * @example Convert tailing subscription into bounded slice
   * ```ts
   * await client.subscribeUpdate({ subscriptionRequestId, locationFilter: locationFilter(LocationFilterType.AbsoluteStartGroupEnd, startGroup, 0n, cutoffGroup), forward: false, priority });
   * ```
   *
   * @example Lower priority only
   * ```ts
   * await client.subscribeUpdate({ subscriptionRequestId, forward: true, priority: 200 });
   * ```
   */
  subscribeUpdate(args: SubscribeUpdateOptions): Promise<void> {
    return subscribeUpdate(this.#session, args)
  }

  /**
   * Switches an active subscription to a different track while retaining the same subscription's
   * output stream.
   *
   * Sends a fresh SUBSCRIBE carrying a SwitchFrom parameter referencing `switchFromRequestId`. A soft
   * switch may keep delivering objects from the old track for a while after SUBSCRIBE_OK, and this can
   * be called again before the previous target has produced anything; only the first object actually
   * delivered by the newest switch target retires everything older.
   *
   * @param args - {@link SwitchOptions} referencing the subscription to switch from and the new subscription options.
   * @returns Either a {@link RequestError} (refusal) or the new request's {@link SubscribeResult}, where `stream`
   * is the same {@link https://developer.mozilla.org/docs/Web/API/ReadableStream | ReadableStream} object returned by the
   * original `subscribe()` call and `done` that subscription's, which resolves once the whole switched chain is over.
   * @throws :{@link MOQtailError} If the client is destroyed.
   * @throws :{@link ProtocolViolationError} If `switchFromRequestId` does not name an active subscription.
   * @throws :{@link InternalError} On transport/control failure (disconnect is triggered before rethrow).
   *
   * @example Switch to a different track
   * ```ts
   * await client.switch({
   *   switchFromRequestId,
   *   switchMode: SwitchMode.Soft,
   *   newSubscribeOptions: { fullTrackName: newTrackName, priority: 0, groupOrder: GroupOrder.Original, locationFilter: locationFilter(LocationFilterType.NextObject) },
   *   onDrainDecision: (request) => true
   * });
   * ```
   */
  switch(args: SwitchOptions): Promise<RequestError | SubscribeResult> {
    return switchTrack(this.#session, args)
  }

  /**
   * Ends a subscription whose publisher has said how many streams it opened and has
   * now delivered all of them, and reports whether that happened.
   *
   * Both halves of that arrive on their own: a PUBLISH_DONE carries the count, and
   * the streams finish whenever they finish. Either can be last, so this is checked
   * from both -- a drained switch target has usually finished its streams well
   * before the PUBLISH_DONE that ends it.
   */
  completeSubscription(request: SubscribeRequest, force = false): boolean {
    return completeSubscription(this.#session, request, force)
  }

  /**
   * One-shot retrieval of a bounded object span of a track, returning a stream of {@link MoqtObject}s.
   *
   * Field highlights (in {@link FetchOptions}):
   * - priority: 0 (highest) .. 255 (lowest); out-of-range rejected.
   * - groupOrder: `GroupOrder.Original` to preserve publisher order; or reorder ascending/descending if supported by server.
   * - locationFilter: Which Objects to fetch. An absolute filter names groups; a relative one counts back from the Largest Object.
   * - parameters: Optional version-specific extension block.
   *
   * Returns either a {@link RequestError} (refusal / invalid request at protocol level) or `{ requestId, fetchOk, stream }` whose `stream`
   * ends naturally after the bounded range completes (no explicit cancel needed for normal completion).
   *
   * Use cases:
   * - Grab a historical window for scrubbing UI while a separate live subscription tails.
   * - Late joiner fetching a short back-buffer then discarding the stream.
   * - Analytics batch job pulling a fixed slice without subscribing long-term.
   *
   * @throws MOQtailError If client is destroyed.
   * @throws ProtocolViolationError Priority out of [0-255].
   * @throws InternalError Transport/control failure (the client disconnects first) then rethrows original error.
   *
   * @remarks
   * - Result stream is finite; reader close occurs automatically when last object delivered.
   * - Use {@link MOQtailClient.fetchCancel} only for early termination; objects already in flight may still arrive.
   *
   * @example Groups 10 through 12, inclusive
   * ```ts
   * const r = await client.fetch({
   *   fullTrackName,
   *   priority: 64,
   *   groupOrder: GroupOrder.Original,
   *   locationFilter: locationFilter(LocationFilterType.AbsoluteStartGroupEnd, 10n, 0n, 12n)
   * })
   * if (!isRequestError(r)) {
   *   for await (const obj of r.stream) {
   *     // consume objects then stream ends automatically
   *   }
   * }
   * ```
   *
   * @example The last 5 groups, counting the current one
   * ```ts
   * const slice = await client.fetch({
   *   fullTrackName,
   *   priority: 32,
   *   groupOrder: GroupOrder.Original,
   *   locationFilter: locationFilter(LocationFilterType.RelativeStart, 5n)
   * })
   * ```
   */
  fetch(args: FetchOptions): Promise<RequestError | FetchResult> {
    return fetch(this.#session, args)
  }

  /**
   * Request early termination of an in‑flight FETCH identified by its `requestId`.
   *
   * Use when the consumer no longer needs the remaining objects (user scrubbed away, UI panel closed, replaced by a new fetch).
   * Resets the fetch's request stream with `CANCELLED` if the id currently maps to an active fetch; otherwise silent no-op (idempotent).
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
   * const r = await client.fetch({ fullTrackName, priority: 32, groupOrder: GroupOrder.Original, locationFilter })
   * if (!isRequestError(r)) {
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
  fetchCancel(requestId: bigint | number): Promise<void> {
    return fetchCancel(this.#session, BigInt(requestId))
  }

  /**
   * Proactively push a track to the relay/peer.
   */
  publish(
    fullTrackName: FullTrackName,
    forward: boolean,
    trackAlias: bigint,
    parameters?: MessageParameters,
    trackProperties?: TrackProperty[],
  ): Promise<RequestError | { requestId: bigint; trackAlias: bigint }> {
    return publish(this.#session, fullTrackName, forward, trackAlias, parameters, trackProperties)
  }

  /**
   * Signals the end of a published track to the peer/relay.
   * * @param publishRequestId - The original requestId used when `publish()` was called.
   */
  publishDone(
    publishRequestId: bigint | number,
    statusCode: number = PublishDoneStatusCode.TrackEnded,
    reasonPhrase: string = 'Track Ended',
  ): Promise<void> {
    return publishDone(this.#session, BigInt(publishRequestId), statusCode, reasonPhrase)
  }

  /**
   * Registers an incoming PUBLISH announcement as a valid data receiver.
   * This prepares the client to ingest pushed data streams matching the published alias.
   * * @param msg - The incoming Publish control message
   * @returns - A stream of MoqtObjects being pushed by the publisher
   */
  acceptPushedTrack(msg: Publish): ReadableStream<MoqtObject> {
    return acceptPushedTrack(this.#session, msg)
  }

  /**
   * Declare (publish) a track namespace to the peer so subscribers using matching prefixes (via {@link MOQtailClient.subscribeNamespace})
   * can discover and begin subscribing/fetching its tracks.
   *
   * Typical flow (publisher side):
   * 1. Prepare / register one or more {@link Track} objects locally (see {@link MOQtailClient.addOrUpdateTrack}).
   * 2. Call `publishNamespace(namespace)` once per namespace prefix to expose those tracks.
   * 3. Later, call {@link MOQtailClient.unpublishNamespace} when no longer publishing under that namespace.
   *
   * Parameter semantics:
   * - trackNamespace: Tuple representing the namespace prefix (e.g. `Tuple.fromUtf8Path('camera/main')`). All tracks whose full names start with this tuple are considered within the announce scope.
   * - parameters: Optional `MessageParameters` added to the PUBLISH_NAMESPACE message.
   *
   * Returns: {@link RequestOk} on success (namespace added to `publishedNamespaces`) or {@link RequestError} explaining refusal.
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
   * - Successful announces are tracked in `publishedNamespaces`; manual removal occurs via {@link MOQtailClient.unpublishNamespace}.
   * - Discovery subscribers (those who issued {@link MOQtailClient.subscribeNamespace}) will receive the resulting {@link PublishNamespace} message.
   *
   * @example Minimal announce
   * ```ts
   * const res = await client.publishNamespace(Tuple.fromUtf8Path('camera/main'))
   * if (!isRequestError(res)) {
   *   // ready to publish objects under tracks with this namespace prefix
   * }
   * ```
   *
   * @example PublishNamespace with an authorization token
   * ```ts
   * const token = { aliasType: TokenAliasType.UseValue, tokenType, tokenValue }
   * const resp = await client.publishNamespace(Tuple.fromUtf8Path('room/1234'), { authorizationToken: [token] })
   * ```
   */
  publishNamespace(trackNamespace: Tuple, parameters?: MessageParameters): Promise<RequestOk | RequestError> {
    return publishNamespace(this.#session, trackNamespace, parameters)
  }

  /**
   * Withdraw a previously announced namespace so new subscribers no longer discover its tracks.
   *
   * Use when shutting down publishing for a logical scope (camera offline, room closed, session ended).
   * Removes the namespace from `publishedNamespaces` locally and closes the stream its PUBLISH_NAMESPACE opened,
   * which is what withdraws the announcement (§3.3.2). This is also how an announce is retracted before the peer
   * has finished processing it.
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
   * Peers that issued {@link MOQtailClient.subscribeNamespace} for a matching prefix should receive the resulting NAMESPACE_DONE.
   * Consider calling this before {@link MOQtailClient.disconnect} to give consumers prompt notice.
   *
   * @example Basic usage
   * ```ts
   * await client.unpublishNamespace(namespace)
   * ```
   *
   * @example Idempotent
   * ```ts
   * await client.unpublishNamespace(namespace) // first time
   * await client.unpublishNamespace(namespace) // no error, already removed
   * ```
   */
  unpublishNamespace(trackNamespace: Tuple): Promise<void> {
    return unpublishNamespace(this.#session, trackNamespace)
  }

  /**
   * Subscribes to namespace announcements under a prefix (§10.18). Discovery only: the
   * peer answers NAMESPACE and NAMESPACE_DONE for matching namespaces, and nothing is
   * subscribed to. To be sent the tracks themselves, use
   * {@link MOQtailClient.subscribeTracks}.
   */
  subscribeNamespace(
    trackNamespacePrefix: Tuple,
    parameters?: MessageParameters,
  ): Promise<{ response: RequestOk | RequestError; cancel: () => Promise<void> }> {
    return subscribeNamespace(this.#session, trackNamespacePrefix, parameters)
  }

  /**
   * Subscribes to every track under a prefix (§10.19): the peer answers with a PUBLISH for
   * each matching track, now and as new ones appear.
   */
  subscribeTracks(
    trackNamespacePrefix: Tuple,
    parameters?: MessageParameters,
  ): Promise<{ response: RequestOk | RequestError; cancel: () => Promise<void> }> {
    return subscribeTracks(this.#session, trackNamespacePrefix, parameters)
  }

  /**
   * Ends a prefix subscription started with {@link MOQtailClient.subscribeNamespace}.
   *
   * @param trackNamespacePrefix - The prefix that subscription was opened with.
   */
  unsubscribeNamespace(trackNamespacePrefix: Tuple): Promise<void> {
    return unsubscribeNamespace(this.#session, trackNamespacePrefix)
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
   * if (!isRequestError(status)) {
   *   // track exists
   * }
   * ```
   */
  trackStatus(
    fullTrackName: FullTrackName,
    trackAlias: bigint,
    parameters?: MessageParameters,
  ): Promise<RequestOk | RequestError> {
    return trackStatus(this.#session, fullTrackName, trackAlias, parameters)
  }

  /**
   * Re-issues one request on a fresh stream after the peer sent a GOAWAY on its request
   * stream (§10.4). Only that request moves: the session, and every other request on it,
   * are left alone.
   *
   * The re-issued request consumes a new Request ID (§10.1), so the opening message is
   * re-stamped with it. The id the caller holds keeps naming the request; only the id on
   * the wire changes.
   *
   * @returns True if the request was re-issued here. A GOAWAY naming a new session URI
   * is not: opening a session is the application's call, so it is surfaced as a
   * `goaway` event and the request stream is closed.
   *
   * @internal
   */
  migrateRequest(requestId: bigint, goAway: GoAway): Promise<boolean> {
    return migrateRequest(this.#session, requestId, goAway)
  }
}
