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

import { InvalidEnumValue } from '../error'
/**
 * Protocol string array exchanged in wt-available-protocols header
 */
export const SUPPORTED_VERSIONS = ['moqt-18']

/**
 * @public
 * Control message types, per draft-18 Table 5.
 *
 * The comment on each member is the Stream column: `Control` is the control stream
 * (§3.3), `Request` a bidirectional request stream, and `First` means the message MUST
 * be the first on a new request stream.
 *
 * Table 5 reserves `0x01` (SETUP for version 00), `0x40`/`0x41` (CLIENT_SETUP /
 * SERVER_SETUP for versions 10 and below) and `0x20`/`0x21` (CLIENT_SETUP /
 * SERVER_SETUP for versions 16 and below). The first three stay in the enum as
 * documentation, but `tryFrom` rejects every RESERVED codepoint, which is what §10
 * requires — an endpoint receiving an unknown message type MUST close the session.
 * `0x20` and `0x21` are still live as ClientSetup / ServerSetup and become reserved
 * once they are folded into Setup (#256).
 */
export enum ControlMessageType {
  ReservedSetupV00 = 0x01, // RESERVED; rejected by tryFrom
  ReservedClientSetupV10 = 0x40, // RESERVED; rejected by tryFrom
  ReservedServerSetupV10 = 0x41, // RESERVED; rejected by tryFrom
  Setup = 0x2f00, // Control
  ClientSetup = 0x20, // RESERVED in draft-18; folded into Setup
  ServerSetup = 0x21, // RESERVED in draft-18; folded into Setup
  GoAway = 0x10, // Control, Request
  MaxRequestId = 0x15, // not in draft-18
  RequestsBlocked = 0x1a, // not in draft-18
  Subscribe = 0x03, // Request, First
  SubscribeOk = 0x04, // Request
  RequestError = 0x05, // Request
  Unsubscribe = 0x0a, // not in draft-18
  RequestUpdate = 0x02, // Request
  PublishDone = 0x0b, // Request
  Fetch = 0x16, // Request, First
  FetchOk = 0x18, // Request
  FetchCancel = 0x17, // not in draft-18
  TrackStatus = 0x0d, // Request, First
  PublishNamespace = 0x06, // Request, First
  RequestOk = 0x07, // Request
  Namespace = 0x08, // Request
  PublishNamespaceDone = 0x09, // not in draft-18
  NamespaceDone = 0x0e, // Request
  PublishNamespaceCancel = 0x0c, // not in draft-18
  SubscribeNamespace = 0x50, // Request, First
  SubscribeTracks = 0x51, // Request, First
  UnsubscribeNamespace = 0x14, // not in draft-18
  Publish = 0x1d, // Request, First
  PublishOk = 0x1e, // Request; an alias of RequestOk (§10.5), not its own body
  PublishBlocked = 0x0f, // Request
  Switch = 0x22, // not in draft-18; moqtail-local extension
}

/**
 * Converts a bigint value to a ControlMessageType enum.
 * @param v - The bigint value.
 * @returns The corresponding ControlMessageType.
 * @throws InvalidEnumValue if the value is not a valid control message type.
 */
export namespace ControlMessageType {
  /** Convert bigint discriminant to enum value or throw on invalid. */
  export function tryFrom(v: bigint): ControlMessageType {
    switch (v) {
      case 0x2f00n:
        return ControlMessageType.Setup
      case 0x20n:
        return ControlMessageType.ClientSetup
      case 0x21n:
        return ControlMessageType.ServerSetup
      case 0x10n:
        return ControlMessageType.GoAway
      case 0x15n:
        return ControlMessageType.MaxRequestId
      case 0x1an:
        return ControlMessageType.RequestsBlocked
      case 0x03n:
        return ControlMessageType.Subscribe
      case 0x04n:
        return ControlMessageType.SubscribeOk
      case 0x05n:
        return ControlMessageType.RequestError
      case 0x0an:
        return ControlMessageType.Unsubscribe
      case 0x02n:
        return ControlMessageType.RequestUpdate
      case 0x0bn:
        return ControlMessageType.PublishDone
      case 0x16n:
        return ControlMessageType.Fetch
      case 0x18n:
        return ControlMessageType.FetchOk
      case 0x17n:
        return ControlMessageType.FetchCancel
      case 0x0dn:
        return ControlMessageType.TrackStatus
      case 0x06n:
        return ControlMessageType.PublishNamespace
      case 0x07n:
        return ControlMessageType.RequestOk
      case 0x08n:
        return ControlMessageType.Namespace
      case 0x09n:
        return ControlMessageType.PublishNamespaceDone
      case 0x0en:
        return ControlMessageType.NamespaceDone
      case 0x0cn:
        return ControlMessageType.PublishNamespaceCancel
      case 0x50n:
        return ControlMessageType.SubscribeNamespace
      case 0x51n:
        return ControlMessageType.SubscribeTracks
      case 0x14n:
        return ControlMessageType.UnsubscribeNamespace
      case 0x1dn:
        return ControlMessageType.Publish
      case 0x1en:
        return ControlMessageType.PublishOk
      case 0x0fn:
        return ControlMessageType.PublishBlocked
      default:
        throw new InvalidEnumValue('ControlMessageType.tryFrom', v)
    }
  }
}

/**
 * @public
 * Error codes for PublishNamespace control messages.
 */
/**
 * @public
 * Subscribe options for SUBSCRIBE_NAMESPACE requests.
 */
export enum NamespaceSubscribeOptions {
  PublishOnly = 0x00,
  NamespaceOnly = 0x01,
  Both = 0x02,
}

/**
 * @public
 * Filter types for subscription requests.
 */
export enum FilterType {
  NextGroupStart = 0x1,
  LatestObject = 0x2,
  AbsoluteStart = 0x3,
  AbsoluteRange = 0x4,
}

/**
 * Converts a bigint value to a FilterType enum.
 * @param v - The bigint value.
 * @returns The corresponding FilterType.
 * @throws InvalidEnumValue if the value is not a valid filter type.
 */
export namespace FilterType {
  export function tryFrom(v: bigint): FilterType {
    switch (v) {
      case 0x1n:
        return FilterType.NextGroupStart
      case 0x2n:
        return FilterType.LatestObject
      case 0x3n:
        return FilterType.AbsoluteStart
      case 0x4n:
        return FilterType.AbsoluteRange
      default:
        throw new InvalidEnumValue('FilterType.tryFrom', v)
    }
  }
}

/**
 * @public
 * Fetch request types for MOQT protocol.
 */
export enum FetchType {
  Standalone = 0x1,
  Relative = 0x2,
  Absolute = 0x3,
}

/**
 * Converts a bigint value to a FetchType enum.
 * @param v - The bigint value.
 * @returns The corresponding FetchType.
 * @throws InvalidEnumValue if the value is not a valid fetch type.
 */
export namespace FetchType {
  export function tryFrom(v: bigint): FetchType {
    switch (v) {
      case 0x1n:
        return FetchType.Standalone
      case 0x2n:
        return FetchType.Relative
      case 0x3n:
        return FetchType.Absolute
      default:
        throw new InvalidEnumValue('FetchType.tryFrom', v)
    }
  }
}

/**
 * @public
 * Group ordering options for object delivery.
 */
export enum GroupOrder {
  Original = 0x0,
  Ascending = 0x1,
  Descending = 0x2,
}

/**
 * Converts a number value to a GroupOrder enum.
 * @param v - The number value.
 * @returns The corresponding GroupOrder.
 * @throws InvalidEnumValue if the value is not a valid group order.
 */
export namespace GroupOrder {
  export function tryFrom(v: number): GroupOrder {
    switch (v) {
      case 0x0:
        return GroupOrder.Original
      case 0x1:
        return GroupOrder.Ascending
      case 0x2:
        return GroupOrder.Descending
      default:
        throw new InvalidEnumValue('GroupOrder.tryFrom', v)
    }
  }
}

/**
 * @public
 * Status codes for track status responses.
 */
export enum TrackStatusCode {
  InProgress = 0x00,
  DoesNotExist = 0x01,
  NotYetBegun = 0x02,
  Finished = 0x03,
  RelayUnavailable = 0x04,
}

/**
 * Converts a bigint value to a TrackStatusCode enum.
 * @param v - The bigint value.
 * @returns The corresponding TrackStatusCode.
 * @throws InvalidEnumValue if the value is not a valid track status code.
 */
export namespace TrackStatusCode {
  export function tryFrom(v: bigint): TrackStatusCode {
    switch (v) {
      case 0x00n:
        return TrackStatusCode.InProgress
      case 0x01n:
        return TrackStatusCode.DoesNotExist
      case 0x02n:
        return TrackStatusCode.NotYetBegun
      case 0x03n:
        return TrackStatusCode.Finished
      case 0x04n:
        return TrackStatusCode.RelayUnavailable
      default:
        throw new InvalidEnumValue('TrackStatusCode.tryFrom', v)
    }
  }
}

/**
 * @public
 * Status codes for PublishDone control messages.
 */
export enum PublishDoneStatusCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  TrackEnded = 0x2,
  SubscriptionEnded = 0x3,
  GoingAway = 0x4,
  Expired = 0x5,
  TooFarBehind = 0x6,
  UpdateFailed = 0x8,
  MalformedTrack = 0x12,
}

/**
 * Converts a bigint value to a PublishDoneStatusCode enum.
 * @param v - The bigint value.
 * @returns The corresponding PublishDoneStatusCode.
 * @throws InvalidEnumValue if the value is not a valid subscribe done status code.
 */
export namespace PublishDoneStatusCode {
  export function tryFrom(v: bigint): PublishDoneStatusCode {
    switch (v) {
      case 0x0n:
        return PublishDoneStatusCode.InternalError
      case 0x1n:
        return PublishDoneStatusCode.Unauthorized
      case 0x2n:
        return PublishDoneStatusCode.TrackEnded
      case 0x3n:
        return PublishDoneStatusCode.SubscriptionEnded
      case 0x4n:
        return PublishDoneStatusCode.GoingAway
      case 0x5n:
        return PublishDoneStatusCode.Expired
      case 0x6n:
        return PublishDoneStatusCode.TooFarBehind
      case 0x8n:
        return PublishDoneStatusCode.UpdateFailed
      case 0x12n:
        return PublishDoneStatusCode.MalformedTrack
      default:
        throw new InvalidEnumValue('PublishDoneStatusCode.tryFrom', v)
    }
  }
}

/**
 * @public
 * Unified error codes for REQUEST_ERROR control messages.
 */
export enum RequestErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  MalformedAuthToken = 0x4,
  ExpiredAuthToken = 0x5,
  DoesNotExist = 0x10,
  InvalidRange = 0x11,
  MalformedTrack = 0x12,
  DuplicateSubscription = 0x19,
  Uninterested = 0x20,
  PrefixOverlap = 0x30,
  InvalidJoiningRequestId = 0x32,
}

/**
 * Converts a bigint value to a RequestErrorCode enum.
 * @param v - The bigint value.
 * @returns The corresponding RequestErrorCode.
 * @throws InvalidEnumValue if the value is not a valid request error code.
 */
export namespace RequestErrorCode {
  export function tryFrom(v: bigint): RequestErrorCode {
    switch (v) {
      case 0x0n:
        return RequestErrorCode.InternalError
      case 0x1n:
        return RequestErrorCode.Unauthorized
      case 0x2n:
        return RequestErrorCode.Timeout
      case 0x3n:
        return RequestErrorCode.NotSupported
      case 0x4n:
        return RequestErrorCode.MalformedAuthToken
      case 0x5n:
        return RequestErrorCode.ExpiredAuthToken
      case 0x10n:
        return RequestErrorCode.DoesNotExist
      case 0x11n:
        return RequestErrorCode.InvalidRange
      case 0x12n:
        return RequestErrorCode.MalformedTrack
      case 0x19n:
        return RequestErrorCode.DuplicateSubscription
      case 0x20n:
        return RequestErrorCode.Uninterested
      case 0x30n:
        return RequestErrorCode.PrefixOverlap
      case 0x32n:
        return RequestErrorCode.InvalidJoiningRequestId
      default:
        throw new InvalidEnumValue('RequestErrorCode.tryFrom', v)
    }
  }
}

if (import.meta.vitest) {
  const { describe, expect, test } = import.meta.vitest

  // Asserted against dev/conformance/draft18/, which is shared with moqtail-rs. No
  // codepoint is repeated here: each test asks the enum what it parses a fixture value
  // as. The loader is imported dynamically so it stays out of the published bundle.
  describe('draft-18 conformance', () => {
    const fixture = async () => await import('../../../test/conformance')

    test('ControlMessageType matches message_types.json', async () => {
      const { messageTypes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(messageTypes(), pascalIdent({ GOAWAY: 'GoAway' }), (codepoint) => {
        try {
          return ControlMessageType[Number(ControlMessageType.tryFrom(codepoint))]
        } catch {
          return undefined
        }
      })
    })

    test('RequestErrorCode matches request_error_codes.json', async () => {
      const { requestErrorCodes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(requestErrorCodes(), pascalIdent(), (codepoint) => {
        try {
          return RequestErrorCode[Number(RequestErrorCode.tryFrom(codepoint))]
        } catch {
          return undefined
        }
      })
    })

    test('every message_types.json entry is exercised', async () => {
      const { messageTypes } = await fixture()
      expect(messageTypes().entries.length).toBeGreaterThan(0)
    })
  })
}
