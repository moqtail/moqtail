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

/**
 * Control messages (moqt-22 §10) are plain data tagged by `kind`. Each one's wire layout
 * is a row of {@link MESSAGES}: its type and its fields in wire order, every field a
 * codec. One encoder and one decoder serve them all.
 */

import { ByteBuffer, FrozenByteBuffer } from './common/byte_buffer'
import type { BaseByteBuffer } from './common/byte_buffer'
import type { Location } from './common/location'
import type { Tuple } from './common/tuple'
import { boundedText, byte, bytes, flag, fullTrackName, location, reason, text, tuple, varint } from './codec'
import type { Codec } from './codec'
import { GroupOrder, PublishDoneStatusCode, RequestErrorCode } from './constants'
import type { FullTrackName } from './data/full_track_name'
import { LengthExceedsMaxError, NotEnoughBytesError, ProtocolViolationError } from './error/error'
import { locationFilterValue, parameters, setupOptions } from './parameter_codec'
import type { LocationFilter, MessageParameters, SetupOptions } from './parameters'
import { TrackProperty } from './property/track_property'

/** @public SETUP (moqt-22 §10.3): the first message each side sends on its control stream. */
export interface Setup {
  kind: 'setup'
  options: SetupOptions
}

/** @public GOAWAY (moqt-22 §10.4): the session, or one request on its own stream, should move. */
export interface GoAway {
  kind: 'goAway'
  /** Where to reconnect; absent means the current URI. */
  newSessionUri?: string
  /** How long (ms) the sender waits before closing; 0 is unspecified. */
  timeout: bigint
}

/** @public SUBSCRIBE (moqt-22 §10.7): asks the publisher for a track. */
export interface Subscribe {
  kind: 'subscribe'
  requestId: bigint
  fullTrackName: FullTrackName
  parameters: MessageParameters
}

/** @public SUBSCRIBE_OK (moqt-22 §10.8): accepts a SUBSCRIBE and names the track's alias. */
export interface SubscribeOk {
  kind: 'subscribeOk'
  trackAlias: bigint
  parameters: MessageParameters
  trackProperties: TrackProperty[]
}

/**
 * @public
 * REQUEST_ERROR (moqt-22 §10.6): refuses a request. A REDIRECT error carries a
 * {@link Redirect}, and no other code does.
 */
export interface RequestError {
  kind: 'requestError'
  errorCode: RequestErrorCode
  /** How long (ms) to wait before retrying, plus one; 0 means do not retry. */
  retryInterval: bigint
  reasonPhrase: string
  redirect?: Redirect
}

/** @public Where a REDIRECT error sends the request (moqt-22 §10.6.1). */
export interface Redirect {
  /** Absent means the current session's URI. */
  connectUri?: string
  /** Empty, together with an empty `trackName`, means the original request's track. */
  trackNamespace: Tuple
  /** Empty for a namespace-scoped request. */
  trackName: Uint8Array
}

/** @public REQUEST_UPDATE (moqt-22 §10.9): changes the parameters of the request on its stream. */
export interface RequestUpdate {
  kind: 'requestUpdate'
  requestId: bigint
  parameters: MessageParameters
}

/** @public PUBLISH_DONE (moqt-22 §10.10): the publisher has finished a subscription. */
export interface PublishDone {
  kind: 'publishDone'
  statusCode: PublishDoneStatusCode
  /** How many data streams the subscription had; {@link UNKNOWN_STREAM_COUNT} when unknown. */
  streamCount: bigint
  errorReason: string
}

/** @public FETCH (moqt-22 §10.12): asks for a bounded span of a track's past Objects. */
export interface Fetch {
  kind: 'fetch'
  requestId: bigint
  fullTrackName: FullTrackName
  parameters: MessageParameters
}

/** @public FETCH_OK (moqt-22 §10.13): accepts a FETCH. */
export interface FetchOk {
  kind: 'fetchOk'
  /** Whether the fetch reaches the end of the track. */
  endOfTrack: boolean
  /** The last location the fetch covers. */
  endLocation: Location
  parameters: MessageParameters
  trackProperties: TrackProperty[]
}

/** @public TRACK_STATUS (moqt-22 §10.15): asks for a track's state without subscribing. */
export interface TrackStatus {
  kind: 'trackStatus'
  requestId: bigint
  trackAlias: bigint
  fullTrackName: FullTrackName
  subscriberPriority: number
  groupOrder: GroupOrder
  forward: boolean
  locationFilter: LocationFilter
  parameters: MessageParameters
}

/** @public PUBLISH_NAMESPACE (moqt-22 §10.16): announces a namespace this side publishes. */
export interface PublishNamespace {
  kind: 'publishNamespace'
  requestId: bigint
  trackNamespace: Tuple
  parameters: MessageParameters
}

/** @public REQUEST_OK (moqt-22 §10.5): accepts a request, PUBLISH_OK included. */
export interface RequestOk {
  kind: 'requestOk'
  parameters: MessageParameters
  /** Only a REQUEST_OK answering TRACK_STATUS may carry them. */
  trackProperties: TrackProperty[]
}

/** @public NAMESPACE (moqt-22 §10.17): a namespace under a subscribed prefix, by its suffix. */
export interface Namespace {
  kind: 'namespace'
  trackNamespaceSuffix: Tuple
}

/** @public NAMESPACE_DONE (moqt-22 §10.17): a namespace under a subscribed prefix is gone. */
export interface NamespaceDone {
  kind: 'namespaceDone'
  trackNamespaceSuffix: Tuple
}

/** @public SUBSCRIBE_NAMESPACE (moqt-22 §10.18): asks to be told of namespaces under a prefix. */
export interface SubscribeNamespace {
  kind: 'subscribeNamespace'
  requestId: bigint
  trackNamespacePrefix: Tuple
  parameters: MessageParameters
}

/** @public SUBSCRIBE_TRACKS (moqt-22 §10.19): asks to be sent every track under a prefix. */
export interface SubscribeTracks {
  kind: 'subscribeTracks'
  requestId: bigint
  trackNamespacePrefix: Tuple
  parameters: MessageParameters
}

/** @public PUBLISH (moqt-22 §10.11): offers a track to the peer without a SUBSCRIBE. */
export interface Publish {
  kind: 'publish'
  requestId: bigint
  fullTrackName: FullTrackName
  trackAlias: bigint
  parameters: MessageParameters
  trackProperties: TrackProperty[]
}

/** @public PUBLISH_SKIPPED (moqt-22 §10.20): a track under a SUBSCRIBE_TRACKS prefix was not offered. */
export interface PublishSkipped {
  kind: 'publishSkipped'
  trackNamespaceSuffix: Tuple
  trackName: Uint8Array
}

/** @public Every control message. */
export type Message =
  | Setup
  | GoAway
  | Subscribe
  | SubscribeOk
  | RequestError
  | RequestUpdate
  | PublishDone
  | Fetch
  | FetchOk
  | TrackStatus
  | PublishNamespace
  | RequestOk
  | Namespace
  | NamespaceDone
  | SubscribeNamespace
  | SubscribeTracks
  | Publish
  | PublishSkipped

/** @public The `kind` of a control message. */
export type MessageKind = Message['kind']

/** @public The stream count of a PUBLISH_DONE whose sender does not know it. */
export const UNKNOWN_STREAM_COUNT = 2n ** 64n - 1n

type Fields<T> = { [K in Exclude<keyof T, 'kind'>]-?: Codec<T[K]> }

interface Layout<T> {
  type: number
  fields: Fields<T>
  /** Checks the decoded message as a whole. */
  check?: (msg: T) => void
}

const MAX_NEW_SESSION_URI_LENGTH = 8192

function layout<T>(type: number, fields: Fields<T>, check?: (msg: T) => void): Layout<T> {
  return check ? { type, fields, check } : { type, fields }
}

/** An unknown code reads as INTERNAL_ERROR (moqt-22 §10.6), so a newer peer is still understood. */
function code<T extends number>(known: object, fallback: T): Codec<T> {
  const values = new Set(Object.values(known))

  return [
    (buf) => {
      const value = buf.getNumberVI()

      return (values.has(value) ? value : fallback) as T
    },
    (buf, value) => buf.putVI(value),
  ]
}

const uriText = boundedText(MAX_NEW_SESSION_URI_LENGTH)
const sessionUri: Codec<string | undefined> = [
  (buf) => uriText[0](buf) || undefined,
  (buf, uri) => uriText[1](buf, uri ?? ''),
]

const redirect: Codec<Redirect | undefined> = [
  (buf) => {
    if (buf.remaining === 0) return undefined

    const connectUri = text[0](buf)
    const trackNamespace = tuple[0](buf)
    const trackName = buf.getLengthPrefixedBytes()

    return connectUri ? { connectUri, trackNamespace, trackName } : { trackNamespace, trackName }
  },
  (buf, value) => {
    if (!value) return

    text[1](buf, value.connectUri ?? '')
    tuple[1](buf, value.trackNamespace)
    buf.putLengthPrefixedBytes(value.trackName)
  },
]

const trackProperties: Codec<TrackProperty[]> = [
  (buf) => TrackProperty.deserializeAll(buf),
  (buf, value) => TrackProperty.serializeInto(value, buf),
]

/** Every control message's type and wire layout, by kind. */
const MESSAGES: { [K in MessageKind]: Layout<Extract<Message, { kind: K }>> } = {
  requestUpdate: layout(0x02, { requestId: varint, parameters }),
  subscribe: layout(0x03, { requestId: varint, fullTrackName, parameters }),
  subscribeOk: layout(0x04, { trackAlias: varint, parameters, trackProperties }),
  requestError: layout(
    0x05,
    {
      errorCode: code<RequestErrorCode>(RequestErrorCode, RequestErrorCode.InternalError),
      retryInterval: varint,
      reasonPhrase: reason,
      redirect,
    },
    (msg) => {
      if ((msg.errorCode === RequestErrorCode.Redirect) !== (msg.redirect !== undefined)) {
        throw new ProtocolViolationError('RequestError', 'a Redirect belongs to a REDIRECT error and to no other code')
      }
    },
  ),
  publishNamespace: layout(0x06, { requestId: varint, trackNamespace: tuple, parameters }),
  requestOk: layout(0x07, { parameters, trackProperties }),
  namespace: layout(0x08, { trackNamespaceSuffix: tuple }),
  publishDone: layout(0x0b, {
    statusCode: code<PublishDoneStatusCode>(PublishDoneStatusCode, PublishDoneStatusCode.InternalError),
    streamCount: varint,
    errorReason: reason,
  }),
  trackStatus: layout(0x0d, {
    requestId: varint,
    trackAlias: varint,
    fullTrackName,
    subscriberPriority: byte,
    groupOrder: byte as Codec<GroupOrder>,
    forward: flag,
    locationFilter: locationFilterValue,
    parameters,
  }),
  namespaceDone: layout(0x0e, { trackNamespaceSuffix: tuple }),
  publishSkipped: layout(0x0f, { trackNamespaceSuffix: tuple, trackName: bytes }),
  goAway: layout(0x10, { newSessionUri: sessionUri, timeout: varint }),
  fetch: layout(0x16, { requestId: varint, fullTrackName, parameters }),
  fetchOk: layout(0x18, { endOfTrack: flag, endLocation: location, parameters, trackProperties }),
  publish: layout(0x1d, { requestId: varint, fullTrackName, trackAlias: varint, parameters, trackProperties }),
  subscribeNamespace: layout(0x50, { requestId: varint, trackNamespacePrefix: tuple, parameters }),
  subscribeTracks: layout(0x51, { requestId: varint, trackNamespacePrefix: tuple, parameters }),
  setup: layout(0x2f00, { options: setupOptions }),
}

const FIRST = new Set<MessageKind>([
  'subscribe',
  'fetch',
  'trackStatus',
  'publishNamespace',
  'subscribeNamespace',
  'subscribeTracks',
  'publish',
])

const KINDS = new Map<number, MessageKind>(
  Object.entries(MESSAGES).map(([kind, { type }]) => [type, kind as MessageKind]),
)
// PUBLISH_OK is an alias of REQUEST_OK (moqt-22 §10.5), not its own body
KINDS.set(0x1e, 'requestOk')

/** @public The wire type of a message kind. */
export function messageType(kind: MessageKind): number {
  return MESSAGES[kind].type
}

/** @public Whether a message of this kind opens a request stream (moqt-22 Table 5, `First`). */
export function isFirst(kind: MessageKind): boolean {
  return FIRST.has(kind)
}

/** @public Whether a request of this kind may be changed by a REQUEST_UPDATE (moqt-22 §10.9). */
export function isUpdatable(kind: MessageKind): boolean {
  return kind !== 'trackStatus' && FIRST.has(kind)
}

/** A REQUEST_OK accepting a request. */
export function requestOk(parameters: MessageParameters = {}, trackProperties: TrackProperty[] = []): RequestOk {
  return { kind: 'requestOk', parameters, trackProperties }
}

/** A REQUEST_ERROR refusing a request, to retry after `retryInterval` (0: do not retry). */
export function requestError(errorCode: RequestErrorCode, reasonPhrase: string, retryInterval = 0n): RequestError {
  return { kind: 'requestError', errorCode, retryInterval, reasonPhrase }
}

/** @public Whether a request's result is the REQUEST_ERROR that refused it. */
export function isRequestError(value: unknown): value is RequestError {
  return (value as Partial<Message> | undefined)?.kind === 'requestError'
}

/** @public The bytes of one control message: its type, payload length, and payload. */
export function encodeMessage(msg: Message): Uint8Array {
  const { type, fields } = MESSAGES[msg.kind] as Layout<Message>
  const payload = new ByteBuffer()

  for (const [key, codec] of Object.entries(fields) as [keyof Message, Codec<unknown>][]) {
    codec[1](payload, msg[key])
  }

  const bytesOut = payload.toUint8Array()
  if (bytesOut.length > 0xffff) throw new LengthExceedsMaxError('encodeMessage', 0xffff, bytesOut.length)

  const buf = new ByteBuffer()
  buf.putVI(type)
  buf.putU16(bytesOut.length)
  buf.putBytes(bytesOut)

  return buf.toUint8Array()
}

/**
 * @public
 * Reads one control message off `buf`.
 *
 * @throws NotEnoughBytesError if `buf` ends before the message does.
 * @throws ProtocolViolationError if the message is malformed, or its type unknown.
 */
export function decodeMessage(buf: BaseByteBuffer): Message {
  const type = buf.getNumberVI()
  const kind = KINDS.get(type)
  if (!kind) throw new ProtocolViolationError('decodeMessage', `unknown message type 0x${type.toString(16)}`)

  const length = buf.getU16()
  if (buf.remaining < length) throw new NotEnoughBytesError('decodeMessage', length, buf.remaining)

  const payload = new FrozenByteBuffer(buf.getBytes(length))
  const { fields, check } = MESSAGES[kind] as Layout<Message>
  const msg: Record<string, unknown> = { kind }

  for (const [key, codec] of Object.entries(fields) as [string, Codec<unknown>][]) msg[key] = codec[0](payload)

  check?.(msg as unknown as Message)

  return msg as unknown as Message
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
  const { Location } = await import('./common/location')
  const { KeyValuePair } = await import('./common/pair')
  const { Tuple } = await import('./common/tuple')
  const { FullTrackName } = await import('./data/full_track_name')
  const { LocationFilterType, SwitchMode, TokenAliasType } = await import('./constants')
  const { locationFilter } = await import('./parameters')

  const hex = (bytes: Uint8Array) => Buffer.from(bytes).toString('hex')
  const decodeHex = (h: string) => decodeMessage(new FrozenByteBuffer(Uint8Array.from(Buffer.from(h, 'hex'))))
  const ftn = FullTrackName.tryNew('room/alice', 'video')
  const ns = Tuple.fromUtf8Path('room/alice')

  // Captured from the class-per-message codec this one replaced: the bytes must not move
  const vectors: [string, Message][] = [
    [
      '03004f040204726f6f6d05616c69636505766964656f0d0281f401050301010203060a020701100701040301060401020105020a1e160301030c01800104010002050304020401010d000c06010203040102',
      {
        kind: 'subscribe',
        requestId: 4n,
        fullTrackName: ftn,
        parameters: {
          objectDeliveryTimeout: 500n,
          authorizationToken: [
            { aliasType: TokenAliasType.UseValue, tokenType: 1n, tokenValue: new Uint8Array([1, 2, 3]) },
          ],
          largestObject: new Location(10n, 2n),
          forward: true,
          subscriberPriority: 7,
          locationFilter: locationFilter(LocationFilterType.AbsoluteRange, 3n, 1n, 9n, 4n),
          groupOrder: GroupOrder.Descending,
          fillParameters: { fillTimeout: 30n, subscriberPriority: 3 },
          switchFrom: { requestId: 12n, mode: SwitchMode.Soft, publishDone: true },
          subgroupFilter: [{ setId: 1, deltas: [[0n, 2n], [5n]] }],
          objectPropertyFilter: [{ setId: 2, propertyType: 4n, deltas: [[1n, 1n]] }],
          includeProperties: false,
          switchingSetAssignment: {
            switchingSetId: 1n,
            algorithmId: 2n,
            throughputThresholdKbps: 3n,
            setThroughputWeight: 4n,
            activateSwitching: 1n,
            setRank: 2,
          },
        },
      },
    ],
    [
      '05001d340002676f0968747470733a2f2f780204726f6f6d05616c6963650101',
      {
        kind: 'requestError',
        errorCode: RequestErrorCode.Redirect,
        retryInterval: 0n,
        reasonPhrase: 'go',
        redirect: { connectUri: 'https://x', trackNamespace: ns, trackName: new Uint8Array([1]) },
      },
    ],
    [
      '0b0007020304646f6e65',
      { kind: 'publishDone', statusCode: PublishDoneStatusCode.TrackEnded, streamCount: 3n, errorReason: 'done' },
    ],
    [
      '0d001902090204726f6f6d05616c69636505766964656f8000000500',
      {
        kind: 'trackStatus',
        requestId: 2n,
        trackAlias: 9n,
        fullTrackName: ftn,
        subscriberPriority: 128,
        groupOrder: GroupOrder.Original,
        forward: false,
        locationFilter: locationFilter(LocationFilterType.NextObject),
        parameters: {},
      },
    ],
    [
      '1000131168747470733a2f2f656c7365776865726505',
      { kind: 'goAway', newSessionUri: 'https://elsewhere', timeout: 5n },
    ],
    ['1000020000', { kind: 'goAway', timeout: 0n }],
    [
      'af00001501042f6d6f71050801076d6f717461696c02020102',
      {
        kind: 'setup',
        options: { path: '/moq', maxFilterRanges: 8n, moqtImplementation: 'moqtail', sstsAlgorithms: [1n, 2n] },
      },
    ],
  ]

  describe('control messages', () => {
    it.each(vectors)('codes %s on the wire', (bytes, msg) => {
      expect(hex(encodeMessage(msg))).toBe(bytes)
      expect(decodeHex(bytes)).toEqual(msg)
    })

    it('keeps the SETUP options it does not know, and writes them back in type order', () => {
      const extensions = [KeyValuePair.tryNewVarInt(0x7f00, 1), KeyValuePair.tryNewBytes(0x7f01, new Uint8Array([2]))]
      const setup: Setup = { kind: 'setup', options: { path: '/moq', extensions } }

      expect(decodeMessage(new FrozenByteBuffer(encodeMessage(setup)))).toEqual(setup)
    })

    it('reads PUBLISH_OK as a REQUEST_OK', () => {
      expect(decodeHex('1e000100')).toEqual({ kind: 'requestOk', parameters: {}, trackProperties: [] })
    })

    it('reads an unknown error code as INTERNAL_ERROR', () => {
      const unknown = 0x3f as RequestErrorCode
      const bytes = encodeMessage({ kind: 'requestError', errorCode: unknown, retryInterval: 0n, reasonPhrase: '' })
      const msg = decodeMessage(new FrozenByteBuffer(bytes)) as RequestError

      expect(msg.errorCode).toBe(RequestErrorCode.InternalError)
    })

    it('refuses a Redirect on any error but REDIRECT', () => {
      const redirect = { trackNamespace: ns, trackName: new Uint8Array() }
      const bytes = encodeMessage({ kind: 'requestError', errorCode: 0, retryInterval: 0n, reasonPhrase: '', redirect })

      expect(() => decodeMessage(new FrozenByteBuffer(bytes))).toThrow(ProtocolViolationError)
    })

    it('waits for the rest of a message cut short, and refuses an unknown type', () => {
      const bytes = encodeMessage({ kind: 'namespace', trackNamespaceSuffix: ns })

      expect(() => decodeMessage(new FrozenByteBuffer(bytes.subarray(0, 5)))).toThrow(NotEnoughBytesError)
      expect(() => decodeHex('3f0000')).toThrow(ProtocolViolationError)
    })

    it('keeps a reason phrase within 1024 bytes', () => {
      const msg: PublishDone = { kind: 'publishDone', statusCode: 0, streamCount: 0n, errorReason: 'x'.repeat(1025) }

      expect(() => encodeMessage(msg)).toThrow(ProtocolViolationError)
    })

    it('tells the request stream openers apart', () => {
      expect(isFirst('subscribe')).toBe(true)
      expect(isFirst('subscribeOk')).toBe(false)
      expect(isUpdatable('trackStatus')).toBe(false)
      expect(isRequestError({ kind: 'requestError' })).toBe(true)
      expect(isRequestError({ requestId: 1n })).toBe(false)
    })
  })
}
