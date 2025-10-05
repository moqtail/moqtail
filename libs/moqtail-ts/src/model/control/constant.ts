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

import { CastingError } from '../error'
import { ClientSetup, ServerSetup } from '../control'
/**
 * 32 bit MOQT Draft-11 version number exchanged in {@link ClientSetup} and {@link ServerSetup}
 */
export const DRAFT_11 = 0xff00000b

/**
 * @public
 * Control message types for MOQT protocol.
 * Each value corresponds to a specific control frame.
 */
export enum ControlMessageType {
  ReservedSetupV00 = 0x01,
  ReservedClientSetupV10 = 0x40,
  ReservedServerSetupV10 = 0x41,
  ClientSetup = 0x20,
  ServerSetup = 0x21,
  GoAway = 0x10,
  MaxRequestId = 0x15,
  RequestsBlocked = 0x1a,
  Subscribe = 0x03,
  SubscribeOk = 0x04,
  SubscribeError = 0x05,
  Unsubscribe = 0x0a,
  SubscribeUpdate = 0x02,
  SubscribeDone = 0x0b,
  Fetch = 0x16,
  FetchOk = 0x18,
  FetchError = 0x19,
  FetchCancel = 0x17,
  TrackStatus = 0x0d,
  TrackStatusOk = 0x0e,
  TrackStatusError = 0x0f,
  PublishNamespace = 0x06,
  PublishNamespaceOk = 0x07,
  PublishNamespaceError = 0x08,
  PublishNamespaceDone = 0x09,
  PublishNamespaceCancel = 0x0c,
  SubscribeNamespace = 0x11,
  SubscribeNamespaceOk = 0x12,
  SubscribeNamespaceError = 0x13,
  UnsubscribeNamespace = 0x14,
}

/**
 * Converts a bigint value to a ControlMessageType enum.
 * @param v - The bigint value.
 * @returns The corresponding ControlMessageType.
 * @throws Error if the value is not a valid control message type.
 */
export function controlMessageTypeFromBigInt(v: bigint): ControlMessageType {
  switch (v) {
    case 0x01n:
      return ControlMessageType.ReservedSetupV00
    case 0x40n:
      return ControlMessageType.ReservedClientSetupV10
    case 0x41n:
      return ControlMessageType.ReservedServerSetupV10
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
      return ControlMessageType.SubscribeError
    case 0x0an:
      return ControlMessageType.Unsubscribe
    case 0x02n:
      return ControlMessageType.SubscribeUpdate
    case 0x0bn:
      return ControlMessageType.SubscribeDone
    case 0x16n:
      return ControlMessageType.Fetch
    case 0x18n:
      return ControlMessageType.FetchOk
    case 0x19n:
      return ControlMessageType.FetchError
    case 0x17n:
      return ControlMessageType.FetchCancel
    case 0x0dn:
      return ControlMessageType.TrackStatus
    case 0x0en:
      return ControlMessageType.TrackStatus
    case 0x06n:
      return ControlMessageType.PublishNamespace
    case 0x07n:
      return ControlMessageType.PublishNamespaceOk
    case 0x08n:
      return ControlMessageType.PublishNamespaceError
    case 0x09n:
      return ControlMessageType.PublishNamespaceDone
    case 0x0cn:
      return ControlMessageType.PublishNamespaceCancel
    case 0x11n:
      return ControlMessageType.SubscribeNamespace
    case 0x12n:
      return ControlMessageType.SubscribeNamespaceOk
    case 0x13n:
      return ControlMessageType.SubscribeNamespaceError
    case 0x14n:
      return ControlMessageType.UnsubscribeNamespace
    default:
      throw new Error(`Invalid ControlMessageType: ${v}`)
  }
}

/**
 * @public
 * Error codes for PublishNamespace control messages.
 */
export enum PublishNamespaceErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  Uninterested = 0x4,
  MalformedAuthToken = 0x10,
  UnknownAuthTokenAlias = 0x11,
  ExpiredAuthToken = 0x12,
}

/**
 * Converts a bigint value to an  enum.
 * @param v - The bigint value.
 * @returns The corresponding PublishNamespaceErrorCode.
 * @throws Error if the value is not a valid announce error code.
 */
export function publishNamespaceErrorCodeFromBigInt(v: bigint): PublishNamespaceErrorCode {
  switch (v) {
    case 0x0n:
      return PublishNamespaceErrorCode.InternalError
    case 0x1n:
      return PublishNamespaceErrorCode.Unauthorized
    case 0x2n:
      return PublishNamespaceErrorCode.Timeout
    case 0x3n:
      return PublishNamespaceErrorCode.NotSupported
    case 0x4n:
      return PublishNamespaceErrorCode.Uninterested
    case 0x10n:
      return PublishNamespaceErrorCode.MalformedAuthToken
    case 0x11n:
      return PublishNamespaceErrorCode.UnknownAuthTokenAlias
    case 0x12n:
      return PublishNamespaceErrorCode.ExpiredAuthToken
    default:
      throw new Error(`Invalid PublishNamespaceErrorCode: ${v}`)
  }
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
 * @throws Error if the value is not a valid filter type.
 */
export function filterTypeFromBigInt(v: bigint): FilterType {
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
      throw new Error(`Invalid FilterType: ${v}`)
  }
}

/**
 * @public
 * Fetch request types for MOQT protocol.
 */
export enum FetchType {
  StandAlone = 0x1,
  Relative = 0x2,
  Absolute = 0x3,
}

/**
 * Converts a bigint value to a FetchType enum.
 * @param v - The bigint value.
 * @returns The corresponding FetchType.
 * @throws CastingError if the value is not a valid fetch type.
 */
export function fetchTypeFromBigInt(v: bigint): FetchType {
  switch (v) {
    case 0x1n:
      return FetchType.StandAlone
    case 0x2n:
      return FetchType.Relative
    case 0x3n:
      return FetchType.Absolute
    default:
      throw new CastingError('fetchTypeFromBigInt', 'bigint', 'FetchType', `Invalid FetchType:${v}`)
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
 * @throws CastingError if the value is not a valid group order.
 */
export function groupOrderFromNumber(v: number): GroupOrder {
  switch (v) {
    case 0x0:
      return GroupOrder.Original
    case 0x1:
      return GroupOrder.Ascending
    case 0x2:
      return GroupOrder.Descending
    default:
      throw new CastingError('groupOrderFromNumber', 'number', 'GroupOrder', `Invalid GroupOrder: ${v}`)
  }
}

/**
 * @public
 * Error codes for Subscribe control messages.
 */
export enum SubscribeErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  TrackDoesNotExist = 0x4,
  InvalidRange = 0x5,
  MalformedAuthToken = 0x10,
  ExpiredAuthToken = 0x12,
}

/**
 * Converts a bigint value to a SubscribeErrorCode enum.
 * @param v - The bigint value.
 * @returns The corresponding SubscribeErrorCode.
 * @throws Error if the value is not a valid subscribe error code.
 */
export function subscribeErrorCodeFromBigInt(v: bigint): SubscribeErrorCode {
  switch (v) {
    case 0x0n:
      return SubscribeErrorCode.InternalError
    case 0x1n:
      return SubscribeErrorCode.Unauthorized
    case 0x2n:
      return SubscribeErrorCode.Timeout
    case 0x3n:
      return SubscribeErrorCode.NotSupported
    case 0x4n:
      return SubscribeErrorCode.TrackDoesNotExist
    case 0x5n:
      return SubscribeErrorCode.InvalidRange
    case 0x10n:
      return SubscribeErrorCode.MalformedAuthToken
    case 0x12n:
      return SubscribeErrorCode.ExpiredAuthToken
    default:
      throw new Error(`Invalid SubscribeErrorCode: ${v}`)
  }
}

/**
 * @public
 * Error codes for Fetch control messages.
 */
export enum FetchErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  TrackDoesNotExist = 0x4,
  InvalidRange = 0x5,
  NoObjects = 0x6,
  InvalidJoiningRequestId = 0x7,
  UnknownStatusInRange = 0x8,
  MalformedTrack = 0x9,
  MalformedAuthToken = 0x10,
  ExpiredAuthToken = 0x12,
}

/**
 * Converts a bigint value to a FetchErrorCode enum.
 * @param v - The bigint value.
 * @returns The corresponding FetchErrorCode.
 * @throws CastingError if the value is not a valid fetch error code.
 */
export function fetchErrorCodeFromBigInt(v: bigint): FetchErrorCode {
  switch (v) {
    case 0x0n:
      return FetchErrorCode.InternalError
    case 0x1n:
      return FetchErrorCode.Unauthorized
    case 0x2n:
      return FetchErrorCode.Timeout
    case 0x3n:
      return FetchErrorCode.NotSupported
    case 0x4n:
      return FetchErrorCode.TrackDoesNotExist
    case 0x5n:
      return FetchErrorCode.InvalidRange
    case 0x6n:
      return FetchErrorCode.NoObjects
    case 0x7n:
      return FetchErrorCode.InvalidJoiningRequestId
    case 0x8n:
      return FetchErrorCode.UnknownStatusInRange
    case 0x9n:
      return FetchErrorCode.MalformedTrack
    case 0x10n:
      return FetchErrorCode.MalformedAuthToken
    case 0x12n:
      return FetchErrorCode.ExpiredAuthToken
    default:
      throw new CastingError('fetchErrorCodeFromBigInt', 'number', 'FetchErrorCode', 'Invalid fetch error code')
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
 * @throws Error if the value is not a valid track status code.
 */
export function trackStatusCodeFromBigInt(v: bigint): TrackStatusCode {
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
      throw new Error(`Invalid TrackStatusCode: ${v}`)
  }
}

/**
 * @public
 * Error codes for SubscribeNamespace control messages.
 */
export enum SubscribeNamespaceErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  NamespacePrefixUnknown = 0x4,
  NamespacePrefixOverlap = 0x5,
  MalformedAuthToken = 0x10,
  ExpiredAuthToken = 0x12,
}

/**
 * Converts a bigint value to a SubscribeNamespaceErrorCode enum.
 * @param v - The bigint value.
 * @returns The corresponding SubscribeNamespaceErrorCode.
 * @throws Error if the value is not a valid subscribe announces error code.
 */
export function subscribeNamespaceErrorCodeFromBigInt(v: bigint): SubscribeNamespaceErrorCode {
  switch (v) {
    case 0x0n:
      return SubscribeNamespaceErrorCode.InternalError
    case 0x1n:
      return SubscribeNamespaceErrorCode.Unauthorized
    case 0x2n:
      return SubscribeNamespaceErrorCode.Timeout
    case 0x3n:
      return SubscribeNamespaceErrorCode.NotSupported
    case 0x4n:
      return SubscribeNamespaceErrorCode.NamespacePrefixUnknown
    case 0x5n:
      return SubscribeNamespaceErrorCode.NamespacePrefixOverlap
    case 0x10n:
      return SubscribeNamespaceErrorCode.MalformedAuthToken
    case 0x12n:
      return SubscribeNamespaceErrorCode.ExpiredAuthToken
    default:
      throw new Error(`Invalid SubscribeNamespaceErrorCode: ${v}`)
  }
}

/**
 * @public
 * Status codes for SubscribeDone control messages.
 */
export enum SubscribeDoneStatusCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  TrackEnded = 0x2,
  SubscriptionEnded = 0x3,
  GoingAway = 0x4,
  Expired = 0x5,
  TooFarBehind = 0x6,
}

/**
 * Converts a bigint value to a SubscribeDoneStatusCode enum.
 * @param v - The bigint value.
 * @returns The corresponding SubscribeDoneStatusCode.
 * @throws Error if the value is not a valid subscribe done status code.
 */
export function subscribeDoneStatusCodeFromBigInt(v: bigint): SubscribeDoneStatusCode {
  switch (v) {
    case 0x0n:
      return SubscribeDoneStatusCode.InternalError
    case 0x1n:
      return SubscribeDoneStatusCode.Unauthorized
    case 0x2n:
      return SubscribeDoneStatusCode.TrackEnded
    case 0x3n:
      return SubscribeDoneStatusCode.SubscriptionEnded
    case 0x4n:
      return SubscribeDoneStatusCode.GoingAway
    case 0x5n:
      return SubscribeDoneStatusCode.Expired
    case 0x6n:
      return SubscribeDoneStatusCode.TooFarBehind
    default:
      throw new Error(`Invalid SubscribeDoneStatusCode: ${v}`)
  }
}
