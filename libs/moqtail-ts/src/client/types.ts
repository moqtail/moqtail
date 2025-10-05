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

import {
  FullTrackName,
  GroupOrder,
  FilterType,
  VersionSpecificParameters,
  FetchType,
  Location,
  SetupParameters,
  ControlMessage,
} from '@/model'
import { PublishNamespaceRequest } from './request/publish_namespace'
import { FetchRequest } from './request/fetch'
import { SubscribeRequest } from './request/subscribe'
import { SubscribeNamespaceRequest } from './request/subscribe_namespace'
import { MOQtailClient } from './client'
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
export type MOQtailRequest = PublishNamespaceRequest | SubscribeNamespaceRequest | FetchRequest | SubscribeRequest

/**
 * Options for {@link MOQtailClient.new} controlling connection target, protocol negotiation, timeouts,
 * and lifecycle callbacks.
 *
 * @example Minimal
 * ```ts
 * const opts: MOQtailClientOptions = {
 *   url: 'https://relay.example.com/moq',
 *   supportedVersions: [0xff00000b]
 * }
 * const client = await MOQtailClient.new(opts)
 * ```
 * @example With callbacks & timeouts
 * ```ts
 * const client = await MOQtailClient.new({
 *   url: relayUrl,
 *   supportedVersions: [0xff00000b],
 *   dataStreamTimeoutMs: 5000,
 *   controlStreamTimeoutMs: 1500,
 *   callbacks: {
 *     onMessageSent: m => logOutbound(m),
 *     onMessageReceived: m => logInbound(m),
 *     onSessionTerminated: r => console.warn('terminated', r)
 *   }
 * })
 * ```
 */
export type MOQtailClientOptions = {
  /** Relay / server endpoint for the underlying {@link https://developer.mozilla.org/docs/Web/API/WebTransport | WebTransport} session (can be absolute {@link https://developer.mozilla.org/en-US/docs/Web/API/URL | URL} or string).*/
  url: string | URL
  /** Ordered preference list of MOQT protocol version numbers (e.g. `0xff00000b`).   */
  supportedVersions: number[]
  /**  {@link SetupParameters} customizations; if omitted a default instance is built.*/
  setupParameters?: SetupParameters
  /**  Passed directly to the browser's {@link https://developer.mozilla.org/docs/Web/API/WebTransport | WebTransport} constructor for {@link https://developer.mozilla.org/docs/Web/API/WebTransportOptions | WebTransportOptions}. */
  transportOptions?: WebTransportOptions
  /** Per *data* uni-stream idle timeout in milliseconds. */
  dataStreamTimeoutMs?: number
  /** Control stream read timeout in milliseconds. */
  controlStreamTimeoutMs?: number
  /** callbacks for observability and logging purposes: */
  callbacks?: {
    /** Called after a control message is successfully written to the {@link ControlStream}. */
    onMessageSent?: (msg: ControlMessage) => void
    /** Called for each incoming control message before protocol handling. */
    onMessageReceived?: (msg: ControlMessage) => void
    /** Fired once when the session ends (normal or error). Receives the reason passed to {@link MOQtailClient.disconnect | disconnect}. */
    onSessionTerminated?: (reason?: unknown) => void
  }
}

/**
 * Parameters for {@link MOQtailClient.subscribe | subscribing} to a track's live objects.
 *
 * @example Latest object
 * ```ts
 * await client.subscribe({
 *   fullTrackName,
 *   priority: 0,
 *   groupOrder: GroupOrder.Original,
 *   forward: true,
 *   filterType: FilterType.LatestObject
 * })
 * ```
 * @example Absolute range
 * ```ts
 * await client.subscribe({
 *   fullTrackName,
 *   priority: 32,
 *   groupOrder: GroupOrder.Original,
 *   forward: true,
 *   filterType: FilterType.AbsoluteRange,
 *   startLocation: { group: 100n, subgroup: 0n, object: 0n },
 *   endGroup: 120n
 * })
 * ```
 */
export type SubscribeOptions = {
  /** Fully qualified track identifier ({@link FullTrackName}). */
  fullTrackName: FullTrackName
  /** Subscriber priority (0 = highest, 255 = lowest). Values outside range are clamped. Fractional values are rounded. */
  priority: number
  /** Desired {@link GroupOrder} (e.g. {@link GroupOrder.Original}) specifying delivery ordering semantics. */
  groupOrder: GroupOrder
  /** If true, deliver objects forward (ascending); if false, reverse/backward semantics (implementation dependent). */
  forward: boolean
  /** {@link FilterType} variant controlling starting subset (e.g. {@link FilterType.LatestObject}). */
  filterType: FilterType
  /** Optional extension {@link VersionSpecificParameters} appended to the SUBSCRIBE control message. */
  parameters?: VersionSpecificParameters
  /** Caller supplied alias for the track (else auto-generated); may be number or bigint. */
  trackAlias?: bigint | number
  /** Required for {@link FilterType.AbsoluteStart} / {@link FilterType.AbsoluteRange}; earliest {@link Location} to include. */
  startLocation?: Location
  /** Required for {@link FilterType.AbsoluteRange}; exclusive upper group boundary (coerced to bigint if number provided). */
  endGroup?: bigint | number
}

/**
 * Narrowing update constraints applied to an existing SUBSCRIBE via {@link MOQtailClient.subscribeUpdate}.
 *
 * Rules: start can only move forward (increase) and endGroup can only move backward (decrease) narrowing the window.
 *
 * @example Narrowing a live window
 * ```ts
 * await client.subscribeUpdate({
 *   requestId,
 *   startLocation: { group: 200n, subgroup: 0n, object: 0n },
 *   endGroup: 210n,
 *   priority: 16,
 *   forward: true
 * })
 * ```
 */
export type SubscribeUpdateOptions = {
  /** The original SUBSCRIBE request id (bigint) being updated. */
  requestId: bigint
  /** New narrowed {@link Location} start. */
  startLocation: Location
  /** New narrowed end group (inclusive / protocol defined) must be \> start group. */
  endGroup: bigint
  /** Updated subscriber priority (same constraints as initial subscribe). 0 is highest, 255 is lowest. */
  priority: number
  /** Updated direction flag. */
  forward: boolean
  /** Optional additional {@link VersionSpecificParameters}; existing parameters persist if omitted. */
  parameters?: VersionSpecificParameters
}

/**
 * Options for {@link MOQtailClient.fetch | performing a FETCH} operation for historical or relative object ranges.
 *
 * @example Standalone fetch
 * ```ts
 * const { requestId, stream } = await client.fetch({
 *   priority: 64,
 *   groupOrder: GroupOrder.Original,
 *   typeAndProps: {
 *     type: FetchType.StandAlone,
 *     props: { fullTrackName, startLocation, endLocation }
 *   }
 * })
 * ```
 * @example Relative fetch joining a subscription
 * ```ts
 * const { requestId: subId } = await client.subscribe({
 *   fullTrackName,
 *   priority: 0,
 *   groupOrder: GroupOrder.Original,
 *   forward: true,
 *   filterType: FilterType.LatestObject
 * })
 * const fetchRes = await client.fetch({
 *   priority: 32,
 *   groupOrder: GroupOrder.Original,
 *   typeAndProps: {
 *     type: FetchType.Relative,
 *     props: { joiningRequestId: subId, joiningStart: 0n }
 *   }
 * })
 * ```
 */
// TODO: Define BaseOptions and extend it with StandAloneOptions, RelativeOptions etc.
// Move the type to top level
export type FetchOptions = {
  /** Request priority (0 = highest, 255 = lowest). Rounded & clamped. */
  priority: number
  /** {@link GroupOrder} governing sequencing. */
  groupOrder: GroupOrder
  /**
   * Discriminated union selecting the {@link FetchType} mode and its specific properties:
   * - StandAlone: full explicit range on a {@link FullTrackName} with start/end {@link Location}s.
   * - Relative / Absolute: join an existing {@link SubscribeRequest} (identified by `joiningRequestId`) with starting position `joiningStart`.
   */
  typeAndProps:
    | {
        /** Standalone historical/segment fetch for a specific {@link FullTrackName}. */
        type: FetchType.StandAlone
        /** Properties for standalone fetch: explicit track and range. */
        props: { fullTrackName: FullTrackName; startLocation: Location; endLocation: Location }
      }
    | {
        /** Fetch a range relative to an existing {@link SubscribeRequest} identified by `joiningRequestId`. */
        type: FetchType.Relative
        /** Properties for relative fetch: subscription id and starting position. */
        props: { joiningRequestId: bigint; joiningStart: bigint }
      }
    | {
        /** Fetch an absolute group/object range relative to a {@link SubscribeRequest}. */
        type: FetchType.Absolute
        /** Properties for absolute fetch: subscription id and starting position. */
        props: { joiningRequestId: bigint; joiningStart: bigint }
      }
  /** Optional {@link VersionSpecificParameters} block. */
  parameters?: VersionSpecificParameters
}
