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

import type {
  EndOfRangeKind,
  FullTrackName,
  GroupOrder,
  LocationFilter,
  MessageParameter,
  Location,
  SetupOptions,
  ControlMessage,
  Datagram,
  SwitchMode,
  GoAway,
  PublishNamespace,
  Publish,
  PublishDone,
  PublishSkipped,
  SubscribeNamespace,
  SubscribeTracks,
  SubscribeOk,
  FetchOk,
  Tuple,
  MoqtObject,
  SubgroupObject,
  SubgroupHeader,
  FetchObject,
  FetchHeader,
} from '@/model'
import type { MOQtailClient } from './client'
import type { DataEvent } from './data_event'
import type { FetchRequest } from '@/client/request/fetch'
import type { PublishRequest } from '@/client/request/publish'
import type { PublishNamespaceRequest } from '@/client/request/publish_namespace'
import type { SubscribeRequest } from '@/client/request/subscribe'
import type { SubscribeNamespaceRequest } from '@/client/request/subscribe_namespace'
import type { TrackStatusRequest } from '@/client/request/track_status'

/**
 * Discriminated union of every in‑flight MOQ‑tail control request tracked by the {@link MOQtailClient}.
 *
 * Each concrete request type encapsulates the original control message plus coordination primitives
 * (e.g. a {@link https://developer.mozilla.org/docs/Web/API/Promise | Promise} facade / stream controller) that resolve when a terminal protocol response
 * (OK / ERROR / CANCEL) is received.
 *
 * Used internally in maps like `MOQtailClient.requests` to look up state by request id without needing
 * multiple heterogeneous collections.
 *
 * Variants:
 * - {@link PublishNamespaceRequest} – pending PUBLISH_NAMESPACE / PUBLISH_NAMESPACE_OK / PUBLISH_NAMESPACE_ERROR.
 * - {@link SubscribeNamespaceRequest} – pending SUBSCRIBE_NAMESPACE sequence.
 * - {@link FetchRequest} – pending FETCH handshake producing a data stream.
 * - {@link SubscribeRequest} – pending SUBSCRIBE producing subgroup object streams.
 *
 * @example Looking up a request by id
 * ```ts
 * function isActive(requests: Map<bigint, MOQtailRequest>, id: bigint) {
 *   const req = requests.get(id)
 *   if (!req) return false
 *   // Narrow by instanceof, e.g. FetchRequest
 *   if (req instanceof FetchRequest) {
 *     console.log('Fetch still pending for', req.message.requestId)
 *   }
 *   return true
 * }
 * ```
 */
export type MOQtailRequest =
  | PublishRequest
  | PublishNamespaceRequest
  | SubscribeNamespaceRequest
  | FetchRequest
  | SubscribeRequest
  | TrackStatusRequest

/**
 * Whatever holds a track alias in the client's routing table: a {@link SubscribeRequest},
 * or the receiver {@link MOQtailClient.acceptPushedTrack} stands up for a track the peer
 * pushes. Incoming objects are named after the holder of the alias their stream carries.
 */
export type TrackAliasHolder = { requestId: bigint; fullTrackName: FullTrackName }

/**
 * Default for {@link MOQtailClientOptions.trackAliasResolutionTimeoutMs}: how long a
 * data stream waits for the control message that establishes its track alias.
 *
 * A subscription's data streams and the SUBSCRIBE_OK naming their alias travel on
 * separate streams, so nothing orders them; the data can arrive first. Waiting costs
 * a stalled reader, giving up costs the whole subgroup, so the wait is generous.
 */
export const DEFAULT_TRACK_ALIAS_RESOLUTION_TIMEOUT_MS = 2000

/**
 * Default for {@link MOQtailClientOptions.switchDrainTimeoutMs}: how long a track a
 * soft switch handed over from may stay quiet before it is retired anyway.
 *
 * A soft switch leaves that track delivering up to the group the new one starts at,
 * so the two meet without a hole, and a PUBLISH_DONE normally ends it. This is the
 * backstop for a publisher that never sends one.
 */
export const DEFAULT_SWITCH_DRAIN_TIMEOUT_MS = 2000

/**
 * Default for {@link MOQtailClientOptions.fillOpenTimeoutMs}: how long after SUBSCRIBE_OK
 * a fill fetch stream may take to open before ordering stops waiting for it.
 */
export const DEFAULT_FILL_OPEN_TIMEOUT_MS = 1000

/**
 * @public
 * The MAX_FILTER_RANGES a client advertises when its setup options set none: how
 * many Range Filter ranges it applies, in total, for one subscription or fetch.
 */
export const DEFAULT_MAX_FILTER_RANGES = 32n

/**
 * Options for the {@link MOQtailClient} constructor: connection target, protocol
 * negotiation, and timeouts. Nothing happens on the network until
 * {@link MOQtailClient.init}.
 *
 * @example
 * ```ts
 * const client = new MOQtailClient({
 *   url: relayUrl,
 *   dataStreamTimeoutMs: 5000,
 *   controlStreamTimeoutMs: 1500,
 * })
 * client.on('sessionterminated', (r) => console.warn('terminated', r))
 * await client.init()
 * ```
 */
export type MOQtailClientOptions = {
  /** Relay / server endpoint for the underlying {@link https://developer.mozilla.org/docs/Web/API/WebTransport | WebTransport} session (can be absolute {@link https://developer.mozilla.org/en-US/docs/Web/API/URL | URL} or string).*/
  url: string | URL
  /**  {@link SetupOptions} customizations; if omitted a default instance is built.*/
  setupOptions?: SetupOptions
  /**  Passed directly to the browser's {@link https://developer.mozilla.org/docs/Web/API/WebTransport | WebTransport} constructor for {@link https://developer.mozilla.org/docs/Web/API/WebTransportOptions | WebTransportOptions}. */
  transportOptions?: WebTransportOptions & { protocols?: string[] }
  /** Per *data* uni-stream idle timeout in milliseconds. */
  dataStreamTimeoutMs?: number
  /** Control stream read timeout in milliseconds. */
  controlStreamTimeoutMs?: number
  /**
   * How long a data stream waits for the control message that establishes its track
   * alias before the stream is abandoned, in milliseconds. Defaults to
   * {@link DEFAULT_TRACK_ALIAS_RESOLUTION_TIMEOUT_MS}.
   */
  trackAliasResolutionTimeoutMs?: number
  /**
   * How long a track a soft switch handed over from may stay quiet before it is
   * retired anyway, in milliseconds. Defaults to
   * {@link DEFAULT_SWITCH_DRAIN_TIMEOUT_MS}.
   */
  switchDrainTimeoutMs?: number
  /**
   * How long after SUBSCRIBE_OK a fill fetch stream may take to open before ordering
   * stops waiting for it, in milliseconds. Defaults to {@link DEFAULT_FILL_OPEN_TIMEOUT_MS}.
   */
  fillOpenTimeoutMs?: number
  /** If true, enables datagram support for the session. */
  enableDatagrams?: boolean
  /** The default {@link ObjectOrdering} of the session's subscriptions and fetches. Without one, objects pass through unordered. */
  ordering?: ObjectOrdering | 'none'
}

/**
 * @public
 * Releases a request's objects in location order, however the publisher maps them to
 * streams or datagrams. An object goes out once its predecessor has; IDs are taken to
 * increase by 1 unless a Prior Group ID Gap or Prior Object ID Gap says otherwise. An
 * object whose predecessor is missing waits within `window`, then the client releases
 * what it holds up to it and reports the skipped locations with `orderinggap`.
 * Requests with descending group order are left unordered.
 */
export interface ObjectOrdering {
  /** How long an object may wait, and how many may be held, before the client stops waiting. */
  window: { durationMs?: number; objects?: number }
  /** What happens to an object that arrives after the client stopped waiting for it: released anyway (default) or dropped. */
  onWindowExceeded?: 'flush' | 'drop'
  /** Object IDs may skip without a Prior Object ID Gap. */
  sparseObjectIds?: boolean
  /** Group IDs may skip without a Prior Group ID Gap. */
  sparseGroupIds?: boolean
}

/**
 * @public
 * Every event a {@link MOQtailClient} emits, keyed by name, with the listener signature
 * each one calls. Attach with `on()`.
 */
export interface ClientEvents {
  /** A GOAWAY arrived: the session is winding down, or a request is moving to a new session. */
  goaway: (msg: GoAway) => void
  /** The session ended, normally or on error. Fires once, with the reason given to `disconnect`. */
  sessionterminated: (reason?: unknown) => void
  /** A control message was written, on the control stream or a request stream. */
  messagesent: (msg: ControlMessage) => void
  /** A control message arrived, before it is handled. */
  messagereceived: (msg: ControlMessage) => void
  /** A data stream header or object was decoded. */
  datareceived: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void
  /** A data stream header or object was written. */
  datasent: (data: SubgroupObject | SubgroupHeader | FetchObject | FetchHeader) => void
  /** A data stream opened, carried an object (identity and size only), or ended. */
  dataevent: (event: DataEvent) => void
  /** A datagram arrived. */
  datagramreceived: (data: Datagram) => void
  /** A datagram was written. */
  datagramsent: (data: Datagram) => void
  /**
   * An ordered request stopped waiting for expected locations: from `from` (inclusive) to
   * `to` (exclusive), the next object it released. See {@link ObjectOrdering}.
   */
  orderinggap: (requestId: bigint, from: Location, to: Location) => void
  /** A PUBLISH_NAMESPACE arrived. */
  namespacepublished: (msg: PublishNamespace) => void
  /**
   * The peer moved the Forward State of a track this side publishes, in its PUBLISH_OK
   * or a later REQUEST_UPDATE. `requestId` is the one {@link MOQtailClient.publish}
   * returned. A publication that declared FORWARD=0 sends nothing until this reports `true`.
   */
  forwardstatechange: (requestId: bigint, forward: boolean) => void
  /**
   * The peer pushed a track. Only one reader can lock `stream`, so listeners check the
   * track first and only the one that wants it reads it.
   */
  peerpublish: (msg: Publish, stream: ReadableStream<MoqtObject>) => void
  /**
   * A PUBLISH_DONE arrived. `requestId` is the request its stream opened: for a pushed
   * track, the `requestId` of the {@link Publish} from `peerpublish`.
   */
  peerpublishdone: (msg: PublishDone, requestId: bigint) => void
  /**
   * The peer has a track matching a {@link MOQtailClient.subscribeTracks} prefix but no
   * stream to send its PUBLISH on yet (§10.20). `prefix` is the subscribed prefix.
   */
  peerpublishskipped: (prefix: Tuple, msg: PublishSkipped) => void
  /** A SUBSCRIBE_NAMESPACE arrived. */
  peersubscribenamespace: (msg: SubscribeNamespace) => void
  /**
   * A SUBSCRIBE_TRACKS arrived. The PUBLISH sent for each matching track states the
   * request's settings from `MessageParameter.publishParametersFor(msg.parameters)`, and
   * goes without Track Properties when `MessageParameter.includesProperties(msg.parameters)`
   * is false.
   */
  peersubscribetracks: (msg: SubscribeTracks) => void
  /** A NAMESPACE arrived on a SUBSCRIBE_NAMESPACE stream. */
  peernamespace: (prefix: Tuple, suffix: Tuple) => void
  /** A NAMESPACE_DONE arrived on a SUBSCRIBE_NAMESPACE stream. */
  peernamespacedone: (prefix: Tuple, suffix: Tuple) => void
}

/**
 * @public
 * What {@link MOQtailClient.subscribe} returns when the subscription is accepted.
 */
export interface SubscribeResult {
  requestId: bigint
  /** The SUBSCRIBE_OK: track alias, parameters, and track properties. */
  subscribeOk: SubscribeOk
  stream: ReadableStream<MoqtObject>
  /**
   * Resolves once the subscription, and every subscription switched from it, is over:
   * with the last one's PUBLISH_DONE, or `undefined` if it ended without one. Never rejects.
   */
  done: Promise<PublishDone | undefined>
}

/**
 * @public
 * What {@link MOQtailClient.fetch} returns when the fetch is accepted.
 */
export interface FetchResult {
  requestId: bigint
  /** The FETCH_OK: end of track, end location, and track properties. */
  fetchOk: FetchOk
  stream: ReadableStream<MoqtObject>
}

/**
 * Called for an End of Range marker on a fetch stream: every Location after the
 * previous Object, if any, up to and including `location`, does not exist, is unknown,
 * or timed out, as `kind` says. Without it, an unknown or timed-out range looks the
 * same as one that does not exist.
 */
export type EndOfRangeHandler = (kind: EndOfRangeKind, location: Location) => void

/**
 * Parameters for {@link MOQtailClient.subscribe | subscribing} to a track's live objects.
 *
 * @example From the next object
 * ```ts
 * await client.subscribe({
 *   fullTrackName,
 *   priority: 0,
 *   groupOrder: GroupOrder.Original,
 *   forward: true,
 *   locationFilter: new LocationFilter(LocationFilterType.NextObject)
 * })
 * ```
 * @example Groups 100 through 120
 * ```ts
 * await client.subscribe({
 *   fullTrackName,
 *   priority: 32,
 *   groupOrder: GroupOrder.Original,
 *   forward: true,
 *   locationFilter: new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 100n, 0n, 120n)
 * })
 * ```
 * @example From the next object, filling the current group from its start
 * ```ts
 * await client.subscribe({
 *   fullTrackName,
 *   priority: 0,
 *   groupOrder: GroupOrder.Original,
 *   locationFilter: new LocationFilter(LocationFilterType.NextObject),
 *   parameters: [new FillParameters([new LocationFilter(LocationFilterType.RelativeStart, 1n)])]
 * })
 * ```
 */
export type SubscribeOptions = {
  /** Fully qualified track identifier ({@link FullTrackName}). */
  fullTrackName: FullTrackName
  /** Subscriber priority (0 = highest, 255 = lowest). Values outside range are clamped. Fractional values are rounded. */
  priority: number
  /** Desired {@link (GroupOrder:enum)} (e.g. {@link (GroupOrder:enum).Original}) specifying delivery ordering semantics. */
  groupOrder: GroupOrder
  /** If present, forward parameter (0x10) is sent with the provided value */
  forward?: boolean
  /** The {@link LocationFilter} selecting which Objects to deliver. Omitted, the subscription is unfiltered. */
  locationFilter?: LocationFilter
  /**
   * Optional extension parameters appended to the SUBSCRIBE control message. A
   * FILL_PARAMETERS here also retrieves already-published Objects on a fill fetch stream.
   */
  parameters?: MessageParameter[]
  /** Per-subscription early discard policy. Overrides the client-level default set via {@link MOQtailClient.setEarlyDiscardPolicy}. */
  earlyDiscardPolicy?: EarlyDiscardPolicyConfig
  /** Called for each End of Range marker on the subscription's fill fetch streams. */
  onEndOfRange?: EndOfRangeHandler
  /** Orders this subscription's objects, its fill included. Overrides {@link MOQtailClientOptions.ordering}. */
  ordering?: ObjectOrdering | 'none'
}

/**
 * Changes applied to an existing SUBSCRIBE via {@link MOQtailClient.subscribeUpdate}.
 *
 * @example Narrowing a live window to groups 200 through 210
 * ```ts
 * await client.subscribeUpdate({
 *   subscriptionRequestId,
 *   locationFilter: new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 200n, 0n, 210n),
 *   priority: 16,
 *   forward: true
 * })
 * ```
 */
export type SubscribeUpdateOptions = {
  /** The original SUBSCRIBE request id (bigint) being updated. */
  subscriptionRequestId: bigint
  /** The new {@link LocationFilter}. Omitted, the subscription keeps its own. */
  locationFilter?: LocationFilter
  /** Updated subscriber priority (same constraints as initial subscribe). 0 is highest, 255 is lowest. Omitted, it's not sent and stays unchanged. */
  priority?: number
  /** Updated direction flag. Omitted, it's not sent and stays unchanged. */
  forward?: boolean
  /** Optional additional parameters; existing parameters persist if omitted. */
  parameters?: MessageParameter[]
}

/**
 * Parameters for {@link MOQtailClient.switch | switching} an existing SUBSCRIBE to a new track.
 *
 * @example Switching a subscription to a new track
 * ```ts
 * await client.switch({
 *   switchFromRequestId,
 *   switchMode: SwitchMode.Soft,
 *   newSubscribeOptions: {
 *     fullTrackName: newFullTrackName,
 *     priority: 0,
 *     groupOrder: GroupOrder.Original,
 *     locationFilter: new LocationFilter(LocationFilterType.NextObject)
 *   }
 * })
 * ```
 */
export type SwitchOptions = {
  /** The original SUBSCRIBE request id (bigint) being switched from. */
  switchFromRequestId: bigint
  /** {@link SwitchMode} controlling how the switch is applied. */
  switchMode: SwitchMode
  /** New subscription options to switch to. No `forward`: the relay controls forwarding during a switch. */
  newSubscribeOptions: Omit<SubscribeOptions, 'forward'>
  /**
   * Called for each object of a superseded subscription that a soft switch left draining.
   * Returning false ends the drain: that subscription is retired and the new track's
   * objects, held until then, are released.
   */
  onDrainDecision?: (request: SubscribeRequest) => boolean
}

/**
 * Configuration for the early discard policy applied to incoming subgroup streams.
 */
export type EarlyDiscardPolicyConfig = {
  /** Cancel a subgroup QUIC stream if it has not fully completed within this many milliseconds. */
  subgroupReceiveTimeout: number
}

/**
 * Options for {@link MOQtailClient.fetch | performing a FETCH} of Objects already published.
 *
 * @example Groups 10 through 12, inclusive
 * ```ts
 * const { requestId, stream } = await client.fetch({
 *   fullTrackName,
 *   priority: 64,
 *   groupOrder: GroupOrder.Original,
 *   locationFilter: new LocationFilter(LocationFilterType.AbsoluteStartGroupEnd, 10n, 0n, 12n)
 * })
 * ```
 * @example The current group and the one before it
 * ```ts
 * await client.fetch({
 *   fullTrackName,
 *   priority: 32,
 *   groupOrder: GroupOrder.Original,
 *   locationFilter: new LocationFilter(LocationFilterType.RelativeStart, 2n)
 * })
 * ```
 */
export type FetchOptions = {
  /** Request priority (0 = highest, 255 = lowest). Rounded & clamped. */
  priority: number
  /** {@link (GroupOrder:enum)} governing sequencing. */
  groupOrder: GroupOrder
  /** Fully qualified track identifier ({@link FullTrackName}). */
  fullTrackName: FullTrackName
  /**
   * Which Objects to fetch; the end, when the filter has one, is the last Object
   * sent. Without a filter the whole track up to the Largest Object is fetched, and
   * a filter without an end also stops at the Largest Object.
   */
  locationFilter?: LocationFilter
  /** Optional parameters block. */
  parameters?: MessageParameter[]
  /** Called for each End of Range marker in the response. */
  onEndOfRange?: EndOfRangeHandler
  /** Orders this fetch's objects. Overrides {@link MOQtailClientOptions.ordering}. */
  ordering?: ObjectOrdering | 'none'
}
