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
/**
 * Protocol string array exchanged in wt-available-protocols header
 */
export const SUPPORTED_VERSIONS = ['moqt-16']

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
  RequestError = 0x05,
  Unsubscribe = 0x0a,
  RequestUpdate = 0x02,
  PublishDone = 0x0b,
  Fetch = 0x16,
  FetchOk = 0x18,
  FetchCancel = 0x17,
  TrackStatus = 0x0d,
  PublishNamespace = 0x06,
  RequestOk = 0x07,
  Namespace = 0x08,
  PublishNamespaceDone = 0x09,
  NamespaceDone = 0x0e,
  PublishNamespaceCancel = 0x0c,
  SubscribeNamespace = 0x11,
  UnsubscribeNamespace = 0x14,
  Publish = 0x1d,
  PublishOk = 0x1e,
  Switch = 0x22,
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
    case 0x11n:
      return ControlMessageType.SubscribeNamespace
    case 0x14n:
      return ControlMessageType.UnsubscribeNamespace
    case 0x1dn:
      return ControlMessageType.Publish
    case 0x1en:
      return ControlMessageType.PublishOk
    default:
      throw new Error(`Invalid ControlMessageType: ${v}`)
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
  Standalone = 0x1,
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
      return FetchType.Standalone
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
 * @throws Error if the value is not a valid subscribe done status code.
 */
export function publishDoneStatusCodeFromBigInt(v: bigint): PublishDoneStatusCode {
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
      throw new Error(`Invalid PublishDoneStatusCode: ${v}`)
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
 * @throws CastingError if the value is not a valid request error code.
 */
export function requestErrorCodeFromBigInt(v: bigint): RequestErrorCode {
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
      throw new CastingError(
        'requestErrorCodeFromBigInt',
        'bigint',
        'RequestErrorCode',
        `Invalid RequestErrorCode: ${v}`,
      )
  }
}
