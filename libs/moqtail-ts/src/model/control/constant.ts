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

import { InvalidEnumValue, StreamResetCode } from '../error'
import { greaseValue } from '../common/grease'
/**
 * Protocol string array exchanged in wt-available-protocols header
 */
export const SUPPORTED_VERSIONS = ['moqt-22']

/**
 * @public
 * Control message types.
 */
export enum ControlMessageType {
  ReservedSetupV00 = 0x01, // RESERVED; rejected by tryFrom
  ReservedClientSetupV10 = 0x40, // RESERVED; rejected by tryFrom
  ReservedServerSetupV10 = 0x41, // RESERVED; rejected by tryFrom
  ReservedClientSetupV16 = 0x20, // RESERVED; rejected by tryFrom
  ReservedServerSetupV16 = 0x21, // RESERVED; rejected by tryFrom
  Setup = 0x2f00, // Control
  GoAway = 0x10, // Control, Request
  Subscribe = 0x03, // Request, First
  SubscribeOk = 0x04, // Request
  RequestError = 0x05, // Request
  RequestUpdate = 0x02, // Request
  PublishDone = 0x0b, // Request
  Fetch = 0x16, // Request, First
  FetchOk = 0x18, // Request
  TrackStatus = 0x0d, // Request, First
  PublishNamespace = 0x06, // Request, First
  RequestOk = 0x07, // Request
  Namespace = 0x08, // Request
  NamespaceDone = 0x0e, // Request
  SubscribeNamespace = 0x50, // Request, First
  SubscribeTracks = 0x51, // Request, First
  Publish = 0x1d, // Request, First
  PublishOk = 0x1e, // Request; an alias of RequestOk (§10.5), not its own body
  PublishSkipped = 0x0f, // Request
}

/**
 * Converts a bigint value to a ControlMessageType enum.
 * @param v - The bigint value.
 * @returns The corresponding ControlMessageType.
 * @throws InvalidEnumValue if the value is not a valid control message type.
 */
export namespace ControlMessageType {
  /**
   * True for the seven types marked `First` in Table 5: each one MUST be the first
   * message on a new bidirectional request stream, and no other type may open one.
   * Every remaining type travels on the control stream or on an already-open request
   * stream.
   */
  export function isFirst(t: ControlMessageType): boolean {
    switch (t) {
      case ControlMessageType.Subscribe:
      case ControlMessageType.Fetch:
      case ControlMessageType.TrackStatus:
      case ControlMessageType.PublishNamespace:
      case ControlMessageType.SubscribeNamespace:
      case ControlMessageType.SubscribeTracks:
      case ControlMessageType.Publish:
        return true
      default:
        return false
    }
  }

  /**
   * True for the six request types §10.9 lets a REQUEST_UPDATE modify. TRACK_STATUS
   * opens a request stream but is not one of them, so an update on its stream is
   * refused with a REQUEST_ERROR.
   */
  export function isUpdatable(t: ControlMessageType): boolean {
    switch (t) {
      case ControlMessageType.Subscribe:
      case ControlMessageType.Publish:
      case ControlMessageType.Fetch:
      case ControlMessageType.PublishNamespace:
      case ControlMessageType.SubscribeNamespace:
      case ControlMessageType.SubscribeTracks:
        return true
      default:
        return false
    }
  }

  /** Convert bigint discriminant to enum value or throw on invalid. */
  export function tryFrom(v: bigint): ControlMessageType {
    switch (v) {
      case 0x2f00n:
        return ControlMessageType.Setup
      case 0x10n:
        return ControlMessageType.GoAway
      case 0x03n:
        return ControlMessageType.Subscribe
      case 0x04n:
        return ControlMessageType.SubscribeOk
      case 0x05n:
        return ControlMessageType.RequestError
      case 0x02n:
        return ControlMessageType.RequestUpdate
      case 0x0bn:
        return ControlMessageType.PublishDone
      case 0x16n:
        return ControlMessageType.Fetch
      case 0x18n:
        return ControlMessageType.FetchOk
      case 0x0dn:
        return ControlMessageType.TrackStatus
      case 0x06n:
        return ControlMessageType.PublishNamespace
      case 0x07n:
        return ControlMessageType.RequestOk
      case 0x08n:
        return ControlMessageType.Namespace
      case 0x0en:
        return ControlMessageType.NamespaceDone
      case 0x50n:
        return ControlMessageType.SubscribeNamespace
      case 0x51n:
        return ControlMessageType.SubscribeTracks
      case 0x1dn:
        return ControlMessageType.Publish
      case 0x1en:
        return ControlMessageType.PublishOk
      case 0x0fn:
        return ControlMessageType.PublishSkipped
      default:
        throw new InvalidEnumValue('ControlMessageType.tryFrom', v)
    }
  }
}

/**
 * @public
 * Filter types for subscription requests.
 */
export enum FilterType {
  NextGroupStart = 0x1,
  LatestObject = 0x2,
  AbsoluteStartFill = 0x3,
  AbsoluteRangeFill = 0x4,
  /** Start Location is `{Largest Object.Group - Relative Previous, 0}`. */
  RelativeStartFill = 0x5,
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
        return FilterType.AbsoluteStartFill
      case 0x4n:
        return FilterType.AbsoluteRangeFill
      case 0x5n:
        return FilterType.RelativeStartFill
      default:
        throw new InvalidEnumValue('FilterType.tryFrom', v)
    }
  }

  /** Whether the publisher also delivers already-published objects on a fill fetch stream. */
  export function isFetchFill(v: FilterType): boolean {
    return (
      v === FilterType.AbsoluteStartFill || v === FilterType.AbsoluteRangeFill || v === FilterType.RelativeStartFill
    )
  }
}

/**
 * @public
 * Switch modes for subscription switching requests.
 */
export enum SwitchMode {
  Hard = 0x0,
  Soft = 0x1,
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
  TooFarBehind = 0x5,
  Expired = 0x6,
  UpdateFailed = 0x8,
  ExcessiveLoad = 0x9,
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
        return PublishDoneStatusCode.TooFarBehind
      case 0x6n:
        return PublishDoneStatusCode.Expired
      case 0x8n:
        return PublishDoneStatusCode.UpdateFailed
      case 0x9n:
        return PublishDoneStatusCode.ExcessiveLoad
      case 0x12n:
        return PublishDoneStatusCode.MalformedTrack
      default:
        throw new InvalidEnumValue('PublishDoneStatusCode.tryFrom', v)
    }
  }

  /**
   * Maps a received status code to a known variant, treating any unknown value
   * (including GREASE) as InternalError. An unknown code is never fatal.
   */
  export function fromWire(v: bigint): PublishDoneStatusCode {
    try {
      return tryFrom(v)
    } catch {
      return PublishDoneStatusCode.InternalError
    }
  }
}

/**
 * @public
 * Unified error codes for REQUEST_ERROR control messages.
 *
 * A separate registry from {@link (StreamResetCode:enum)}, which disagrees with it on
 * the same names: `GOING_AWAY` is `0x6` here but `0x4` as a stream reset code.
 */
export enum RequestErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  MalformedAuthToken = 0x4,
  ExpiredAuthToken = 0x5,
  GoingAway = 0x6,
  ExcessiveLoad = 0x9,
  DoesNotExist = 0x10,
  InvalidRange = 0x11,
  MalformedTrack = 0x12,
  DuplicateSubscription = 0x19,
  Uninterested = 0x20,
  PrefixOverlap = 0x30,
  NamespaceTooLarge = 0x31,
  InvalidSwitch = 0x32,
  UnsupportedExtension = 0x33,
  Redirect = 0x34,
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
      case 0x6n:
        return RequestErrorCode.GoingAway
      case 0x9n:
        return RequestErrorCode.ExcessiveLoad
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
      case 0x31n:
        return RequestErrorCode.NamespaceTooLarge
      case 0x32n:
        return RequestErrorCode.InvalidSwitch
      case 0x33n:
        return RequestErrorCode.UnsupportedExtension
      case 0x34n:
        return RequestErrorCode.Redirect
      default:
        throw new InvalidEnumValue('RequestErrorCode.tryFrom', v)
    }
  }

  /**
   * Maps a received error code to a known variant, treating any unknown value
   * (including GREASE) as InternalError. An unknown error code is never fatal and
   * never closes the session.
   */
  export function fromWire(v: bigint): RequestErrorCode {
    try {
      return tryFrom(v)
    } catch {
      return RequestErrorCode.InternalError
    }
  }
}
